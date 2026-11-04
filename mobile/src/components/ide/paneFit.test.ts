import { describe, expect, it } from "vitest";

import { EDITOR_MIN_WIDTH, fitPanes } from "./paneFit";

describe("fitPanes", () => {
  it("keeps the stored widths when the editor has room", () => {
    expect(fitPanes(1600, 260, 340)).toEqual({ rail: 260, agent: 340, editor: 1600 - 260 - 340 - 2 });
  });

  it("gives up agent width first, then rail width, never below the minimums", () => {
    const narrow = fitPanes(1024, 260, 340);
    expect(narrow.editor).toBeGreaterThanOrEqual(EDITOR_MIN_WIDTH - 1);
    expect(narrow.agent).toBeLessThan(340);
    const tiny = fitPanes(900, 260, 340);
    expect(tiny.agent).toBe(280);
    expect(tiny.rail).toBeLessThan(260);
    expect(fitPanes(600, 260, 340).rail).toBe(220);
  });
});
