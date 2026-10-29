import { describe, expect, it } from "vitest";

import type { Attachment } from "../components/chat/attach";
import { parseDraftAttachments, persistableAttachments } from "./drafts";

const att = (over: Partial<Attachment>): Attachment => ({ id: "a", name: "a.pdf", image: false, state: "done", path: "up/a.pdf", ...over });

describe("draft attachments", () => {
  it("keeps only finished uploads and drops blob previews", () => {
    const kept = persistableAttachments([
      att({}),
      att({ id: "b", state: "uploading", path: undefined }),
      att({ id: "c", state: "error", path: undefined }),
      att({ id: "d", image: true, uri: "blob:http://x/1" }),
      att({ id: "e", image: true, uri: "file:///tmp/e.png" }),
    ]);
    expect(kept.map((a) => a.id)).toEqual(["a", "d", "e"]);
    expect(kept[1].uri).toBeUndefined();
    expect(kept[2].uri).toBe("file:///tmp/e.png");
  });

  it("restores what was stored and tolerates corrupt storage", () => {
    const raw = JSON.stringify(persistableAttachments([att({})]));
    expect(parseDraftAttachments(raw)).toEqual([att({})]);
    expect(parseDraftAttachments("nope")).toEqual([]);
    expect(parseDraftAttachments(JSON.stringify([{ id: 1 }]))).toEqual([]);
    expect(parseDraftAttachments(null)).toEqual([]);
  });
});
