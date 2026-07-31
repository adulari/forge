import { describe, expect, it } from "vitest";

import {
  compareTerminalIds,
  MAX_TERMINALS_PER_SESSION,
  nextTerminalId,
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
