import { describe, expect, it } from "vitest";

import type { NoteTodo } from "../../lib/apiNotes";
import {
  EMPTY_PAD,
  noteProjectKey,
  noteProjectName,
  noteProjects,
  padCaption,
  padReducer,
  splitTodos,
  threadSeed,
  type PadState,
} from "./notesModel";

const todo = (id: string, text: string, done = false): NoteTodo => ({
  id,
  text,
  done,
  session_id: null,
  created_at: 1,
  updated_at: 1,
});

describe("noteProjectKey", () => {
  it("maps a worktree session to its checkout and drops trailing slashes", () => {
    expect(noteProjectKey("/w/app/.forge/worktrees/fix-1")).toBe("/w/app");
    expect(noteProjectKey("/w/app/")).toBe("/w/app");
    expect(noteProjectKey("/")).toBe("/");
  });
  it("names a project by its last path segment", () => {
    expect(noteProjectName("/w/app")).toBe("app");
    expect(noteProjectName("/")).toBe("/");
  });
});

describe("noteProjects", () => {
  it("lists the session's project first and one row per checkout", () => {
    const rows = noteProjects("/w/app/.forge/worktrees/x", [{ path: "/w/other" }, { path: "/w/app" }], [{ cwd: "/w/third" }]);
    expect(rows.map((r) => r.path)).toEqual(["/w/app", "/w/other", "/w/third"]);
    expect(rows[0].name).toBe("app");
  });
  it("copes with no session", () => {
    expect(noteProjects(null, [{ path: "/w/a" }]).map((r) => r.path)).toEqual(["/w/a"]);
    expect(noteProjects(null, [])).toEqual([]);
  });
});

describe("splitTodos", () => {
  it("separates open from finished items and keeps order", () => {
    const { open, done } = splitTodos([todo("a", "one"), todo("b", "two", true), todo("c", "three")]);
    expect(open.map((t) => t.id)).toEqual(["a", "c"]);
    expect(done.map((t) => t.id)).toEqual(["b"]);
  });
});

describe("threadSeed", () => {
  it("seeds the session with the item and titles it with the first line", () => {
    expect(threadSeed(todo("a", "  Fix the flaky test\nit fails on CI  "))).toEqual({
      title: "Fix the flaky test",
      prompt: "Fix the flaky test\nit fails on CI",
    });
  });
  it("shortens a long title", () => {
    const { title } = threadSeed(todo("a", "x".repeat(200)));
    expect(title.length).toBe(60);
    expect(title.endsWith("…")).toBe(true);
  });
});

describe("padReducer", () => {
  const loaded = padReducer(EMPTY_PAD, { type: "server", body: "hello", updatedAt: 10 });

  it("adopts the server copy while nothing is being edited", () => {
    expect(loaded).toMatchObject({ draft: "hello", savedBody: "hello", baseUpdatedAt: 10, status: "clean" });
    expect(padReducer(loaded, { type: "server", body: "newer", updatedAt: 20 }).draft).toBe("newer");
  });

  it("keeps an unsaved edit and flags a conflicting server change", () => {
    const editing = padReducer(loaded, { type: "edit", text: "hello world" });
    expect(editing.status).toBe("dirty");
    const hit = padReducer(editing, { type: "server", body: "from the phone", updatedAt: 30 });
    expect(hit.draft).toBe("hello world");
    expect(hit.conflict).toEqual({ body: "from the phone", updated_at: 30 });
    expect(padCaption(hit)).toBe("Edited on another device");
  });

  it("does not call our own save a conflict", () => {
    const editing = padReducer(loaded, { type: "edit", text: "hello world" });
    const saved = padReducer(padReducer(editing, { type: "saving" }), { type: "saved", body: "hello world", updatedAt: 40 });
    expect(saved).toMatchObject({ status: "clean", baseUpdatedAt: 40, conflict: null });
    expect(padReducer(saved, { type: "server", body: "hello world", updatedAt: 40 })).toBe(saved);
  });

  it("stays dirty when typing continued during the save", () => {
    let state: PadState = padReducer(loaded, { type: "edit", text: "a" });
    state = padReducer(state, { type: "saving" });
    state = padReducer(state, { type: "edit", text: "ab" });
    state = padReducer(state, { type: "saved", body: "a", updatedAt: 50 });
    expect(state.status).toBe("dirty");
    expect(state.baseUpdatedAt).toBe(50);
  });

  it("ignores a refetch that races our own save", () => {
    let state: PadState = padReducer(loaded, { type: "edit", text: "ab" });
    state = padReducer(state, { type: "saving" });
    state = padReducer(state, { type: "edit", text: "abc" });
    const raced = padReducer(state, { type: "server", body: "a", updatedAt: 40 });
    expect(raced).toBe(state);
    expect(raced.conflict).toBeNull();
  });

  it("resolves a conflict either way", () => {
    const conflicted = padReducer(padReducer(loaded, { type: "edit", text: "mine" }), {
      type: "conflict",
      conflict: { body: "theirs", updated_at: 60 },
    });
    expect(padReducer(conflicted, { type: "useTheirs" })).toMatchObject({ draft: "theirs", status: "clean", baseUpdatedAt: 60 });
    const kept = padReducer(conflicted, { type: "keepMine" });
    expect(kept).toMatchObject({ draft: "mine", status: "dirty", baseUpdatedAt: 60, conflict: null });
  });

  it("captions each state", () => {
    expect(padCaption(EMPTY_PAD)).toBe("Nothing saved yet");
    expect(padCaption(loaded)).toBe("Saved");
    expect(padCaption(padReducer(loaded, { type: "edit", text: "x" }))).toBe("Unsaved changes");
    expect(padCaption(padReducer(loaded, { type: "failed" }))).toMatch(/Could not save/);
  });
});
