import { describe, expect, it } from "vitest";

import { breakpointFor, isNarrowDesktop, layoutBreakpointFor } from "./breakpoint";

describe("breakpoints", () => {
  it("splits compact, medium and expanded at 640 and 1024", () => {
    expect(breakpointFor(639)).toBe("compact");
    expect(breakpointFor(640)).toBe("medium");
    expect(breakpointFor(1023)).toBe("medium");
    expect(breakpointFor(1024)).toBe("expanded");
  });

  it("keeps the desktop app on the workbench layout when the window is narrowed", () => {
    expect(layoutBreakpointFor(800, true)).toBe("expanded");
    expect(layoutBreakpointFor(640, true)).toBe("expanded");
    expect(isNarrowDesktop(800, true)).toBe(true);
    expect(isNarrowDesktop(1200, true)).toBe(false);
  });

  it("leaves phone widths and non-desktop targets alone", () => {
    expect(layoutBreakpointFor(400, true)).toBe("compact");
    expect(layoutBreakpointFor(800, false)).toBe("medium");
    expect(isNarrowDesktop(800, false)).toBe(false);
  });
});
