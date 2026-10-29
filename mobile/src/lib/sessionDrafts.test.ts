import { describe, expect, it } from "vitest";

import type { Attachment } from "../components/chat/attach";
import { type DraftMap, withDraftAttachments, withDraftText } from "./sessionDrafts";

const uploading = (id: string): Attachment => ({ id, name: `${id}.png`, image: true, state: "uploading", uri: `file:///${id}` });

describe("session drafts", () => {
  it("keeps a finished upload in the thread it started in after a session switch", () => {
    let drafts: DraftMap = {};
    drafts = withDraftAttachments(drafts, "a", [uploading("x")]);
    drafts = withDraftText(drafts, "b", "typing in b");
    drafts = withDraftAttachments(drafts, "a", (prev) => prev.map((item) => (item.id === "x" ? { ...item, state: "done", path: "/up/x.png" } : item)));
    expect(drafts.a.attachments).toEqual([{ ...uploading("x"), state: "done", path: "/up/x.png" }]);
    expect(drafts.b).toEqual({ text: "typing in b", attachments: [] });
  });

  it("adds a pick that resolves late to its own thread without touching the visible draft", () => {
    let drafts: DraftMap = withDraftText({}, "b", "visible text");
    drafts = withDraftAttachments(drafts, "a", (prev) => [...prev, uploading("late")]);
    expect(drafts.a.attachments.map((item) => item.id)).toEqual(["late"]);
    expect(drafts.b.text).toBe("visible text");
    expect(drafts.b.attachments).toEqual([]);
  });

  it("preserves the text when only attachments change, and the reverse", () => {
    let drafts = withDraftText({}, "a", "hello");
    drafts = withDraftAttachments(drafts, "a", [uploading("x")]);
    expect(drafts.a.text).toBe("hello");
    drafts = withDraftText(drafts, "a", "hello world");
    expect(drafts.a.attachments).toHaveLength(1);
  });

  it("does not mutate the previous map", () => {
    const before: DraftMap = withDraftText({}, "a", "one");
    const after = withDraftAttachments(before, "a", [uploading("x")]);
    expect(before.a.attachments).toEqual([]);
    expect(after).not.toBe(before);
  });
});
