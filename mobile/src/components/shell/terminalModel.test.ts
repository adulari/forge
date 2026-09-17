import { describe, expect, it } from "vitest";

import {
  compareTerminalIds,
  MAX_TERMINALS_PER_SESSION,
  nextTerminalId,
  TERMINAL_BACKSPACE,
  terminalInputDelta,
  terminalTitle,
} from "./terminalModel";

describe("terminal model", () => {
  it("sorts generated terminal ids numerically", () => {
    expect(["term-10", "term-2", "term-1"].sort(compareTerminalIds)).toEqual([
      "term-1",
      "term-2",
      "term-10",
    ]);
  });

  it("reuses the first free generated id", () => {
    expect(nextTerminalId(["term-1", "term-3"])).toBe("term-2");
  });

  it("honours the daemon terminal cap", () => {
    const ids = Array.from(
      { length: MAX_TERMINALS_PER_SESSION },
      (_, index) => `custom-${index}`,
    );
    expect(nextTerminalId(ids)).toBeNull();
  });

  it("formats generated and custom titles", () => {
    expect(terminalTitle("term-4")).toBe("Terminal 4");
    expect(terminalTitle("build")).toBe("build");
  });
});

describe("terminal input delta", () => {
  it("sends nothing for a no-op change", () => {
    expect(terminalInputDelta("ec", "ec")).toBe("");
    expect(terminalInputDelta("", "")).toBe("");
  });

  it("sends only the appended tail when the native buffer never actually clears", () => {
    // Reproduces the reported bug one keystroke at a time: typing "echo" delivered the whole
    // accumulated buffer on every change ("e", "ec", "ech", "echo") because the native EditText
    // was never really reset back to "" between keystrokes.
    let previous = "";
    const sent: string[] = [];
    for (const next of ["e", "ec", "ech", "echo"]) {
      const delta = terminalInputDelta(previous, next);
      if (delta) sent.push(delta);
      previous = next;
    }
    expect(sent).toEqual(["e", "c", "h", "o"]);
    expect(sent.join("")).toBe("echo");
  });

  it("sends only the new character when the native buffer DOES clear between keystrokes", () => {
    // The other valid timing: each onChangeText delivers just the fresh character because the
    // previous imperative reset landed in time. Neither is a prefix of the other, so the
    // fallback sends `next` as-is — which is exactly right here.
    expect(terminalInputDelta("e", "c")).toBe("c");
  });

  it("maps a shrinking buffer to one DEL per character removed (an onKeyPress backspace miss)", () => {
    expect(terminalInputDelta("echo", "ech")).toBe(TERMINAL_BACKSPACE);
    expect(terminalInputDelta("echo", "e")).toBe(TERMINAL_BACKSPACE.repeat(3));
  });

  it("falls back to sending the new text verbatim for a non-incremental edit", () => {
    expect(terminalInputDelta("foo", "bar")).toBe("bar");
  });
});
