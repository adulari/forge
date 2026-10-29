import { describe, expect, it } from "vitest";

import { chordErrorStatus, describeChordStatus } from "./globalChordCore";

describe("global chord status", () => {
  it("keeps the typed kind and message the shell sends", () => {
    expect(chordErrorStatus("Alt+Space", { kind: "unavailable", message: "Another application is already using this shortcut." })).toEqual({
      state: "error",
      accelerator: "Alt+Space",
      kind: "unavailable",
      message: "Another application is already using this shortcut.",
    });
    expect(chordErrorStatus("Alt+Space", { kind: "unsupported", message: "No Wayland." })).toMatchObject({ kind: "unsupported" });
  });

  it("degrades unknown rejections to unavailable with a readable message", () => {
    expect(chordErrorStatus("Alt+Space", "plugin not found")).toMatchObject({ kind: "unavailable", message: "plugin not found" });
    expect(chordErrorStatus("Alt+Space", new Error("x"))).toMatchObject({ kind: "unavailable", message: "x" });
    expect(chordErrorStatus("Alt+Space", undefined)).toMatchObject({ kind: "unavailable", message: "The system refused this shortcut." });
    expect(chordErrorStatus("Alt+Space", { kind: "weird" })).toMatchObject({ kind: "unavailable" });
  });

  it("describes only what is worth saying", () => {
    expect(describeChordStatus({ state: "idle" })).toBeNull();
    expect(describeChordStatus({ state: "registered", accelerator: "Alt+Space" })).toEqual({ tone: "ok", text: "Also works from any app." });
    const taken = describeChordStatus(chordErrorStatus("Alt+Space", { kind: "unavailable", message: "Taken." }));
    expect(taken?.tone).toBe("warn");
    expect(taken?.text).toContain("Taken.");
  });
});
