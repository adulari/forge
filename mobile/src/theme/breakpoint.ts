// DESIGN_SYSTEM.md §7. Pure so it can be tested without react-native.
export type Breakpoint = "compact" | "medium" | "expanded";

export const COMPACT_MAX = 640;
export const EXPANDED_MIN = 1024;

/** compact <640 · medium 640-1023 · expanded >=1024 (window width, pt). */
export function breakpointFor(width: number): Breakpoint {
  return width < COMPACT_MAX ? "compact" : width < EXPANDED_MIN ? "medium" : "expanded";
}

/**
 * The desktop app keeps its workbench chrome (sidebar rail, docks, split panes) at every window
 * width that is not phone-sized, so a narrowed window collapses the sidebar instead of falling
 * back to the phone tab layout. Web and mobile keep the 1024 threshold: a browser tab that
 * narrow is a small screen, not a small window.
 */
export function layoutBreakpointFor(width: number, desktopApp: boolean): Breakpoint {
  const bp = breakpointFor(width);
  return desktopApp && bp === "medium" ? "expanded" : bp;
}

/** The desktop app below the full-width threshold shows the sidebar as a rail. */
export function isNarrowDesktop(width: number, desktopApp: boolean): boolean {
  return desktopApp && breakpointFor(width) === "medium";
}
