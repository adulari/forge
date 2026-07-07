// DESIGN_SYSTEM.md §7 — same screens, phone -> desktop.
import { useWindowDimensions } from "react-native";

export type Breakpoint = "compact" | "medium" | "expanded";

export interface BreakpointInfo {
  bp: Breakpoint;
  width: number;
  isCompact: boolean;
  isExpanded: boolean;
}

/** compact <640 · medium 640-1023 · expanded >=1024 (window width, pt). */
export function useBreakpoint(): BreakpointInfo {
  const { width } = useWindowDimensions();
  const bp: Breakpoint = width < 640 ? "compact" : width < 1024 ? "medium" : "expanded";
  return { bp, width, isCompact: bp === "compact", isExpanded: bp === "expanded" };
}
