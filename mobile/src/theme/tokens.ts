// DESIGN_SYSTEM.md §1 (Color) + §3 (space/shape/depth), verbatim.
// This is the ONLY file in src/theme allowed to contain raw hex color literals —
// every other theme module imports tokens from here instead of inlining hex.
import { Platform } from "react-native";

// ---------------------------------------------------------------------------
// §1.1 Ember scale (brand, shared by both themes)
// ---------------------------------------------------------------------------

export interface EmberScale {
  ember100: string;
  ember200: string;
  ember300: string;
  ember400: string;
  ember500: string;
  ember600: string;
  ember700: string;
  ember900: string;
}

const emberScale: EmberScale = {
  ember100: "#FFE9D2",
  ember200: "#FFCE9E",
  ember300: "#FFAF66",
  ember400: "#FF913C",
  ember500: "#F5761A",
  ember600: "#C75D10",
  ember700: "#9C480C",
  ember900: "#4A2206",
};

// ---------------------------------------------------------------------------
// §1.2 / §1.3 Semantic color tokens (one shape, two instances: dark + light)
// ---------------------------------------------------------------------------

export interface ColorTokens {
  bg0: string;
  bg1: string;
  bg2: string;
  bg3: string;
  borderStrong: string;
  border: string;
  ink: string;
  ink2: string;
  ink3: string;
  ink4: string;
  accent: string;
  accentPressed: string;
  onAccent: string;
  success: string;
  danger: string;
  warn: string;
  info: string;
  successBg: string;
  dangerBg: string;
  warnBg: string;
  /** Text color for content painted on `warnBg` (dark theme banner note). */
  warnBgInk: string;
  selection: string;
  overlayScrim: string;
  ember: EmberScale;
  /** Accent with zero alpha for native gradients (avoids transparent-black interpolation). */
  accentTransparent: string;
  /** Native approximation opacity for the top ambient ember wash. */
  forgeWashOpacity: number;
  /** Move 1 (thermal identity) — HeatEdge gradient start (ember400). */
  heatEdgeFrom: string;
  /** Move 1 (thermal identity) — HeatEdge gradient end (ember500). */
  heatEdgeTo: string;
  /** Move 1 — HeatEdge outward glow shadow color (ember-tinted, low alpha). */
  heatGlow: string;
  /** Move 1 — StatusDot busy-state radial halo behind the dot. */
  dotGlow: string;
  /** Move 1 — Screen's single top ambient ember wash (web CSS radial-gradient string). */
  forgeWash: string;
  /** Hearth — de-boxed list row separator (§ "De-boxed lists"): a translucent hairline,
   * NOT the solid `border` used on card edges. */
  hairline: string;
  /** Hearth — HeatEdge "waiting" gradient start (a pending decision, not just running). */
  waitingEdgeFrom: string;
  /** Hearth — HeatEdge "waiting" gradient end. */
  waitingEdgeTo: string;
  /** Hearth — HeatEdge "waiting" glow shadow color. Zero-alpha on light (the paper theme's
   * waiting edge carries no glow — see "Fleet · Light" / "Chat · Light" screens). */
  waitingGlow: string;
  /** Keyboard focus-visible ring (web) — low-alpha accent so tabbing reads as a quiet
   * hairline, never a solid box. */
  focusRing: string;
}

export const darkTokens: ColorTokens = {
  bg0: "#0B0B10",
  bg1: "#131318",
  bg2: "#1B1B22",
  bg3: "#24242D",
  borderStrong: "#34343E",
  border: "#26262E",
  ink: "#E9E9EF",
  ink2: "#A9A9B6",
  ink3: "#6E6E7A",
  ink4: "#4A4A55",
  accent: emberScale.ember400,
  accentPressed: emberScale.ember500,
  onAccent: "#1B1B22",
  success: "#7DD394",
  danger: "#F0716E",
  warn: "#EDBD52",
  info: "#4FD0D9",
  successBg: "#12291A",
  dangerBg: "#2E1516",
  warnBg: "#33270F",
  warnBgInk: "#FFD9A8",
  selection: "#2E2415",
  overlayScrim: "rgba(8,8,12,0.6)",
  accentTransparent: "rgba(255,145,60,0)",
  forgeWashOpacity: 0.05,
  ember: emberScale,
  heatEdgeFrom: emberScale.ember400,
  heatEdgeTo: emberScale.ember500,
  heatGlow: "rgba(255,145,60,0.22)",
  dotGlow: "rgba(255,145,60,0.18)",
  forgeWash: "radial-gradient(1100px 420px at 50% -8%, rgba(255,145,60,0.05), transparent 62%)",
  hairline: "rgba(38,38,46,0.6)",
  waitingEdgeFrom: "#F0716E",
  waitingEdgeTo: "#C24845",
  waitingGlow: "rgba(240,113,110,0.25)",
  focusRing: "rgba(255,145,60,0.45)",
};

export const lightTokens: ColorTokens = {
  bg0: "#F1EEE8",
  bg1: "#FAF8F4",
  bg2: "#FFFFFF",
  bg3: "#F3F0EA",
  borderStrong: "#D6D2C8",
  border: "#E7E3DA",
  ink: "#211F1B",
  ink2: "#57544C",
  ink3: "#8B8779",
  ink4: "#B4B0A3",
  accent: emberScale.ember600,
  accentPressed: emberScale.ember700,
  onAccent: "#FFFFFF",
  success: "#1E8A47",
  danger: "#C93835",
  warn: "#9A6E0C",
  info: "#0E7C86",
  // Hearth handoff value (was #E4F4E7) — "Fleet · Light" / "Chat · Light" screens.
  successBg: "#E4F3E7",
  dangerBg: "#FBE7E5",
  warnBg: "#F7EED3",
  // §1.3 has no ink override for warnBg — the default `ink` already reads fine
  // on the paper-toned warnBg, unlike dark's near-black warnBg.
  warnBgInk: "#211F1B",
  selection: "#F6E7D2",
  overlayScrim: "rgba(30,26,20,0.35)",
  accentTransparent: "rgba(199,93,16,0)",
  forgeWashOpacity: 0.04,
  ember: emberScale,
  heatEdgeFrom: emberScale.ember400,
  heatEdgeTo: emberScale.ember500,
  // Hearth light: the "Fleet · Light" prototype paints the running edge with no outward glow
  // (paper surfaces carry no ambient ember light) — zero-alpha keeps the gradient only.
  heatGlow: "rgba(199,93,16,0)",
  dotGlow: "rgba(199,93,16,0.16)",
  forgeWash: "radial-gradient(1100px 420px at 50% -8%, rgba(199,93,16,0.04), transparent 62%)",
  // Hearth "hairline" is already exactly this theme's `border` value — the light palette
  // never needed the dark theme's translucency trick (paper hairlines are already subtle).
  hairline: "#E7E3DA",
  // Hearth "Fleet · Light" / "Chat · Light": the waiting edge reuses this theme's own
  // danger scale (not the dark-fixed #F0716E — the light palette re-derives every
  // semantic color, see HANDOFF.md's separate Light token block) and carries no glow.
  waitingEdgeFrom: "#C93835",
  waitingEdgeTo: "#9C2D2A",
  waitingGlow: "rgba(201,56,53,0)",
  focusRing: "rgba(199,93,16,0.45)",
};

// ---------------------------------------------------------------------------
// §1.4 Fixed semantic mapping (never swap)
// ---------------------------------------------------------------------------

export type StatusDotState = "idle" | "busy" | "waiting" | "done";

export function statusDotColor(state: StatusDotState, tokens: ColorTokens): string {
  switch (state) {
    case "busy":
      return tokens.accent;
    case "waiting":
      return tokens.danger;
    case "done":
      return tokens.ink4;
    case "idle":
    default:
      return tokens.ink3;
  }
}

/** Context gauge fill color: accent below 70%, warn 70-90%, danger above 90%. */
export function gaugeColor(pct: number, tokens: ColorTokens): string {
  if (pct > 90) return tokens.danger;
  if (pct > 70) return tokens.warn;
  return tokens.accent;
}

// ---------------------------------------------------------------------------
// §3 Space, shape, depth
// ---------------------------------------------------------------------------

export interface SpaceScale {
  space2: number;
  space4: number;
  space8: number;
  space12: number;
  space16: number;
  space20: number;
  space24: number;
  space32: number;
  space48: number;
}

export const space: SpaceScale = {
  space2: 2,
  space4: 4,
  space8: 8,
  space12: 12,
  space16: 16,
  space20: 20,
  space24: 24,
  space32: 32,
  space48: 48,
};

/** Screen gutter: 16 (compact), 24 (medium+). Pair with useBreakpoint(). */
export const gutter = { compact: 16, medium: 24 } as const;

export const cardPadding = { x: 12, y: 14 } as const;

/** List row min-height 56; dense rows 44. */
export const rowHeight = { list: 56, dense: 44 } as const;

/** Tap targets must be >= 44x44. */
export const tapTarget = 44;

export interface RadiiScale {
  radius4: number;
  radius8: number;
  radius12: number;
  radius16: number;
  radiusPill: number;
  /** Hearth — Segmented track (outer). Distinct from `radius8`/`radius12`: Segmented is the
   * one control with its own outer/inner pair, not the general button/card scale. */
  radiusSegmentOuter: number;
  /** Hearth — Segmented thumb (inner). */
  radiusSegmentInner: number;
}

export const radii: RadiiScale = {
  radius4: 4,
  radius8: 8,
  radius12: 12,
  radius16: 16,
  radiusPill: 999,
  radiusSegmentOuter: 10,
  radiusSegmentInner: 7,
};

export interface ShadowStyle {
  shadowColor: string;
  shadowOpacity: number;
  shadowRadius: number;
  shadowOffset: { width: number; height: number };
  elevation: number;
}

export interface DepthTokens {
  /** The one shadow both themes use: palette/sheet elevation. */
  sheet: ShadowStyle;
  /** Ambient shadow on raised surfaces — light theme only; null on dark (hairlines only). */
  raised: ShadowStyle | null;
}

export const depthDark: DepthTokens = {
  sheet: {
    shadowColor: "#000000",
    shadowOpacity: 0.35,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 8 },
    elevation: 24,
  },
  raised: null,
};

export const depthLight: DepthTokens = {
  sheet: {
    shadowColor: "#000000",
    shadowOpacity: 0.35,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 8 },
    elevation: 24,
  },
  raised: {
    shadowColor: "#1E1A14",
    shadowOpacity: 0.06,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
};

function hexToRgba(hex: string, alpha: number): string {
  const h = hex.replace("#", "");
  const r = parseInt(h.substring(0, 2), 16);
  const g = parseInt(h.substring(2, 4), 16);
  const b = parseInt(h.substring(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Cross-platform shadow: RN Web warns that `shadow*`/`elevation` props are deprecated in
 * favor of `boxShadow`, but native (iOS/Android) has no `boxShadow` support at all — only
 * web gets the CSS translation, native keeps the RN shadow props verbatim.
 */
export function shadowStyle(s: ShadowStyle): Record<string, unknown> {
  if (Platform.OS === "web") {
    const color = s.shadowColor.startsWith("#") ? hexToRgba(s.shadowColor, s.shadowOpacity) : s.shadowColor;
    return { boxShadow: `${s.shadowOffset.width}px ${s.shadowOffset.height}px ${s.shadowRadius}px ${color}` };
  }
  return {
    shadowColor: s.shadowColor,
    shadowOpacity: s.shadowOpacity,
    shadowRadius: s.shadowRadius,
    shadowOffset: s.shadowOffset,
    elevation: s.elevation,
  };
}
