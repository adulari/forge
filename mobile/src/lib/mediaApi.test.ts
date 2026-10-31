import { describe, expect, it } from "vitest";

import { evidenceImageUrl, screenshotPathOf, workspaceImageUrl, workspacePdfUrl } from "./mediaApi";

describe("media urls", () => {
  it("encodes the session, path and thumbnail width", () => {
    expect(workspaceImageUrl("http://h/tok", "s1", "docs/a b.png", 320)).toBe(
      "http://h/tok/api/workspace/image?session=s1&path=docs%2Fa+b.png&w=320",
    );
    expect(evidenceImageUrl("http://h/tok", "s1", "0001-browser.png")).toBe(
      "http://h/tok/api/evidence/image?session=s1&file=0001-browser.png",
    );
  });
});

describe("screenshotPathOf", () => {
  it("reads the path out of a screenshot tool result", () => {
    expect(
      screenshotPathOf("screenshot written to /s/evidence/x/0001-browser.png (4210 bytes; recorded as session evidence)"),
    ).toBe("/s/evidence/x/0001-browser.png");
  });

  it("reads a path with spaces and ignores other results", () => {
    expect(screenshotPathOf("screenshot written to /a b/c.png (12 bytes)")).toBe("/a b/c.png");
    expect(screenshotPathOf("exit 0")).toBeNull();
    expect(screenshotPathOf(null)).toBeNull();
  });
});

describe("workspacePdfUrl", () => {
  it("names the session and path", () => {
    expect(workspacePdfUrl("http://h/tok", "s1", "docs/a b.pdf")).toBe(
      "http://h/tok/api/workspace/pdf?session=s1&path=docs%2Fa+b.pdf",
    );
  });
});
