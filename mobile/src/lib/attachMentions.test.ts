import { describe, expect, it } from "vitest";

describe("upload mentions with spaces in the path", () => {
  it("strips a braced mention and finds its path", async () => {
    const { stripAttachMentions, uploadMentionPaths, LEADING_ATTACH_MENTION_RE } = await import("./attachMentions");
    const row = "@{/home/f/Trading Bots/x/.forge/uploads/s/1-a.png} @/r/.forge/uploads/s/2-b.txt\nhi there";
    expect(stripAttachMentions(row)).toBe("hi there");
    const lead = row.match(LEADING_ATTACH_MENTION_RE)?.[1] ?? "";
    expect(uploadMentionPaths(lead)).toEqual(["/home/f/Trading Bots/x/.forge/uploads/s/1-a.png", "/r/.forge/uploads/s/2-b.txt"]);
  });
});
