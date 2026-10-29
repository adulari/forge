import { describe, expect, it } from "vitest";

import {
  advanceActivity,
  closesItselfOnExit,
  compareTerminalIds,
  copyOrInterrupt,
  EMPTY_TERMINAL_ACTIVITY,
  exitLabel,
  hasUnseenOutput,
  LARGE_BUFFER_LINES,
  measuredCharWidth,
  neighbourTerminal,
  resizeDelay,
  terminalGrid,
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

describe("copy or interrupt", () => {
  const base = { meta: false, hasSelection: false, now: 10_000, lastCopyAt: null };

  it("interrupts on Ctrl+C with nothing selected and ignores a bare Cmd+C", () => {
    expect(copyOrInterrupt(base)).toBe("interrupt");
    expect(copyOrInterrupt({ ...base, meta: true })).toBe("ignore");
  });

  it("copies a selection, then interrupts when pressed again inside 500 ms", () => {
    expect(copyOrInterrupt({ ...base, hasSelection: true })).toBe("copy");
    expect(copyOrInterrupt({ ...base, hasSelection: true, lastCopyAt: 9_600 })).toBe("interrupt");
    expect(copyOrInterrupt({ ...base, hasSelection: true, meta: true, lastCopyAt: 9_600 })).toBe("interrupt");
  });

  it("copies again once the window has passed", () => {
    expect(copyOrInterrupt({ ...base, hasSelection: true, lastCopyAt: 9_499 })).toBe("copy");
    expect(copyOrInterrupt({ ...base, hasSelection: true, lastCopyAt: 9_500 })).toBe("interrupt");
  });
});

describe("terminal geometry", () => {
  it("derives the grid from the measured glyph and honours the minimum", () => {
    const input = { width: 800, height: 320, charWidth: 7.2, lineHeight: 16, paddingX: 14 };
    expect(terminalGrid(input)).toEqual({ cols: 107, rows: 20 });
    expect(terminalGrid({ ...input, width: 60, height: 20 })).toEqual({ cols: 20, rows: 4 });
  });

  it("only trusts a measured glyph width that is plausible", () => {
    expect(measuredCharWidth(264, 40, 6.6)).toBeCloseTo(6.6);
    expect(measuredCharWidth(0, 40, 6.6)).toBe(6.6);
    expect(measuredCharWidth(Number.NaN, 40, 6.6)).toBe(6.6);
    expect(measuredCharWidth(4000, 40, 6.6)).toBe(6.6);
  });

  it("holds a resize back only for a large buffer", () => {
    expect(resizeDelay(LARGE_BUFFER_LINES - 1)).toBe(0);
    expect(resizeDelay(LARGE_BUFFER_LINES)).toBeGreaterThan(0);
  });
});

describe("terminal activity", () => {
  const running = (id: string, sequence: number) => ({ terminal_id: id, status: "running" as const, sequence });

  it("does not flag output that predates the first look", () => {
    const first = advanceActivity(EMPTY_TERMINAL_ACTIVITY, [running("term-1", 40), running("term-2", 9)], "term-1");
    expect(hasUnseenOutput(first, "term-2", "term-1")).toBe(false);
    expect(first.streaming["term-2"]).toBe(false);
  });

  it("flags a background terminal that printed, and clears when it becomes the active one", () => {
    const first = advanceActivity(EMPTY_TERMINAL_ACTIVITY, [running("term-1", 4), running("term-2", 9)], "term-1");
    const second = advanceActivity(first, [running("term-1", 4), running("term-2", 12)], "term-1");
    expect(hasUnseenOutput(second, "term-2", "term-1")).toBe(true);
    expect(second.streaming["term-2"]).toBe(true);
    const third = advanceActivity(second, [running("term-1", 4), running("term-2", 12)], "term-1");
    expect(hasUnseenOutput(third, "term-2", "term-1")).toBe(true);
    expect(third.streaming["term-2"]).toBe(false);
    const looked = advanceActivity(third, [running("term-1", 4), running("term-2", 12)], "term-2");
    expect(hasUnseenOutput(looked, "term-2", "term-2")).toBe(false);
  });

  it("never shows an exited terminal as streaming", () => {
    const first = advanceActivity(EMPTY_TERMINAL_ACTIVITY, [running("term-2", 1)], "term-1");
    const second = advanceActivity(first, [{ terminal_id: "term-2", status: "exited", sequence: 5 }], "term-1");
    expect(second.streaming["term-2"]).toBe(false);
    expect(hasUnseenOutput(second, "term-2", "term-1")).toBe(true);
  });

  it("forgets a terminal the daemon no longer lists", () => {
    const first = advanceActivity(EMPTY_TERMINAL_ACTIVITY, [running("term-1", 1), running("term-2", 1)], "term-1");
    const second = advanceActivity(first, [running("term-1", 1)], "term-1");
    expect(Object.keys(second.latest)).toEqual(["term-1"]);
  });
});

describe("exited shells", () => {
  it("closes the tab on a clean exit only", () => {
    expect(closesItselfOnExit(0)).toBe(true);
    expect(closesItselfOnExit(3)).toBe(false);
    expect(closesItselfOnExit(null)).toBe(false);
    expect(closesItselfOnExit(undefined)).toBe(false);
  });

  it("labels the exit with its code when it has one", () => {
    expect(exitLabel(3)).toBe("exited (3)");
    expect(exitLabel(null)).toBe("exited");
  });

  it("selects the tab that slides into the closed one's place, else the one before", () => {
    const ids = ["term-1", "term-2", "term-3"];
    expect(neighbourTerminal(ids, "term-2")).toBe("term-3");
    expect(neighbourTerminal(ids, "term-3")).toBe("term-2");
    expect(neighbourTerminal(ids, "term-1")).toBe("term-2");
    expect(neighbourTerminal(["term-1"], "term-1")).toBeNull();
    expect(neighbourTerminal(ids, "term-9")).toBeNull();
  });
});
