import { describe, expect, it } from "vitest";

import { safeExternalHref } from "./linkSafety";

describe("assistant link safety", () => {
  it("allows ordinary web, mail, and fragment links", () => {
    expect(safeExternalHref("https://forge.example/docs")).toBe("https://forge.example/docs");
    expect(safeExternalHref("mailto:hello@example.com")).toBe("mailto:hello@example.com");
    expect(safeExternalHref("#details")).toBe("#details");
  });

  it("rejects executable and privileged schemes", () => {
    expect(safeExternalHref("javascript:alert(1)")).toBeNull();
    expect(safeExternalHref("file:///etc/passwd")).toBeNull();
    expect(safeExternalHref("data:text/html,pwned")).toBeNull();
  });
});
