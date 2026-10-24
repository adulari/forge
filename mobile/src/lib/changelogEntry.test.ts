import { describe, expect, it } from "vitest";

import { releaseForApp, splitEntry } from "./changelogEntry";

describe("splitEntry", () => {
  it("separates the bold summary from the detail", () => {
    expect(splitEntry("**Auto-edit ran `rm -rf ~`.** It now asks first.")).toEqual({
      lead: "Auto-edit ran `rm -rf ~`.",
      detail: "It now asks first.",
    });
  });

  it("treats a bullet without a bold summary as all summary", () => {
    expect(splitEntry("Plain entry with `code`.")).toEqual({ lead: "Plain entry with `code`.", detail: "" });
  });

  it("keeps multi-line detail", () => {
    expect(splitEntry("**Lead.**\nline one\nline two").detail).toBe("line one\nline two");
  });
});

describe("releaseForApp", () => {
  const releases = [
    { version: "2.21.0", date: "2026-10-18", entries: [] },
    { version: "2.20.0", date: "2026-10-10", entries: [] },
  ];

  it("returns null when the daemon has no section for the desktop version", () => {
    expect(releaseForApp(releases, "2.23.1", true)).toBeNull();
  });

  it("finds the matching section on a daemon that has it", () => {
    expect(releaseForApp(releases, "2.20.0", true)?.version).toBe("2.20.0");
  });

  it("falls back to the newest release for mobile builds", () => {
    expect(releaseForApp(releases, "1.0.1", false)?.version).toBe("2.21.0");
  });
});
