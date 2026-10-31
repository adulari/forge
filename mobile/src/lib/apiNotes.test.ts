import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "./api";
import { addNoteTodo, getProjectNotes, saveScratchpad, scratchpadConflict, updateNoteTodo } from "./apiNotes";

const transport = vi.hoisted(() => ({ tFetch: vi.fn() }));

vi.mock("./transport", () => ({ tFetch: transport.tFetch, TWebSocket: class {} }));

const reply = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("project notes api", () => {
  beforeEach(() => transport.tFetch.mockReset());

  it("reads a project's notes with the path encoded", async () => {
    transport.tFetch.mockResolvedValue(reply({ project: "/w/my app", body: "", updated_at: 0, todos: [] }));
    await getProjectNotes("https://forge.test/tok", "/w/my app");
    expect(transport.tFetch.mock.calls[0][0]).toBe("https://forge.test/tok/api/notes?project=%2Fw%2Fmy%20app");
  });

  it("saves the scratchpad on top of the updated_at it was read at", async () => {
    transport.tFetch.mockResolvedValue(reply({ ok: true, updated_at: 5 }));
    await saveScratchpad("https://forge.test/tok", "/w/app", "hello", 3);
    const [url, init] = transport.tFetch.mock.calls[0];
    expect(url).toBe("https://forge.test/tok/api/notes");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body)).toEqual({ project: "/w/app", body: "hello", expected_updated_at: 3 });
  });

  it("posts to-do changes to their own routes", async () => {
    transport.tFetch.mockImplementation(async () => reply({ id: "td_1" }));
    await addNoteTodo("https://forge.test/tok", "/w/app", "ship it");
    await updateNoteTodo("https://forge.test/tok", "td_1", { done: true });
    expect(transport.tFetch.mock.calls[0][0]).toBe("https://forge.test/tok/api/notes/todos");
    expect(JSON.parse(transport.tFetch.mock.calls[0][1].body)).toEqual({ project: "/w/app", text: "ship it" });
    expect(transport.tFetch.mock.calls[1][0]).toBe("https://forge.test/tok/api/notes/todos/td_1/update");
    expect(JSON.parse(transport.tFetch.mock.calls[1][1].body)).toEqual({ done: true });
  });

  it("recognises a conflicting save and nothing else", async () => {
    transport.tFetch.mockResolvedValue(reply({ error: "changed", code: "conflict", body: "theirs", updated_at: 9 }, 409));
    const error = await saveScratchpad("https://forge.test/tok", "/w/app", "mine", 1).catch((e: unknown) => e);
    expect(scratchpadConflict(error)).toEqual({ body: "theirs", updated_at: 9 });
    expect(scratchpadConflict(new ApiError(500, "boom"))).toBeNull();
    expect(scratchpadConflict(new ApiError(409, "other", { error: "x" }))).toBeNull();
    expect(scratchpadConflict(new Error("net"))).toBeNull();
  });
});
