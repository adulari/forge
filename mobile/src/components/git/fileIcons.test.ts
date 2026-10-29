import { describe, expect, it } from "vitest";

import { fileMark } from "./fileIcons";

describe("fileMark", () => {
  it("colours by language", () => {
    expect(fileMark("src/lib.rs")).toEqual({ tag: "RS", hue: "#D9733B" });
    expect(fileMark("a/b/App.tsx").tag).toBe("TSX");
    expect(fileMark("a/b/App.tsx").hue).toBe(fileMark("x.ts").hue);
  });

  it("is case-insensitive and ignores directory dots", () => {
    expect(fileMark("dir.d/README.MD").tag).toBe("MD");
  });

  it("names a few well-known files", () => {
    expect(fileMark("Cargo.lock").tag).toBe("LOCK");
    expect(fileMark("deploy/Dockerfile").tag).toBe("DOCK");
  });

  it("falls back to a neutral tag from the extension", () => {
    expect(fileMark("notes.xyzzy")).toEqual({ tag: "XYZ", hue: "#8A8F98" });
    expect(fileMark("LICENSE").tag).toBe("·");
  });
});
