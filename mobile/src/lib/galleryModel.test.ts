import { describe, expect, it } from "vitest";

import { galleryEntries } from "./galleryModel";

const item = (file: string, over: object = {}) => ({
  file,
  ts: "2026-10-29T10:00:00Z",
  source: "browser",
  caption: null,
  url: null,
  size: 10,
  ...over,
});

describe("galleryEntries", () => {
  it("lists newest first and names the tool", () => {
    const entries = galleryEntries([item("0001-browser.png"), item("0002-device.png", { source: "device" })]);
    expect(entries.map((e) => e.file)).toEqual(["0002-device.png", "0001-browser.png"]);
    expect(entries.map((e) => e.tool)).toEqual(["Device", "Browser"]);
  });

  it("titles a shot by its caption, then its page, then its file", () => {
    const [a, b, c] = galleryEntries([
      item("c.png"),
      item("b.png", { url: "https://example.com/cart" }),
      item("a.png", { caption: "  empty cart " }),
    ]);
    expect(a.title).toBe("empty cart");
    expect(b.title).toBe("example.com/cart");
    expect(c.title).toBe("c.png");
  });
});
