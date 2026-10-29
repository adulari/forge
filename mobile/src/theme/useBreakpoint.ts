// DESIGN_SYSTEM.md §7 — same screens, phone -> desktop.
import { useWindowDimensions } from "react-native";

import { isTauri } from "../lib/platform";
import { breakpointFor, isNarrowDesktop, layoutBreakpointFor, type Breakpoint } from "./breakpoint";

export type { Breakpoint } from "./breakpoint";

export interface BreakpointInfo {
  bp: Breakpoint;
  width: number;
  isCompact: boolean;
  isExpanded: boolean;
  /** Desktop app narrower than the full workbench width: expanded chrome, sidebar as a rail. */
  isNarrowDesktop: boolean;
}

/** compact <640 · medium 640-1023 · expanded >=1024 (window width, pt); the desktop app treats
 * medium as expanded (see `layoutBreakpointFor`). */
export function useBreakpoint(): BreakpointInfo {
  const { width } = useWindowDimensions();
  const bp = layoutBreakpointFor(width, isTauri);
  return {
    bp,
    width,
    isCompact: bp === "compact",
    isExpanded: bp === "expanded",
    isNarrowDesktop: isNarrowDesktop(width, isTauri),
  };
}

export { breakpointFor };
