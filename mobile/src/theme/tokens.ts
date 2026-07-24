// mobile/redesign/DESIGN_SYSTEM.md §1 (Color) + §3 (space/shape/depth), verbatim.
// Machined — supersedes Emberline. This is the ONLY file in src/theme allowed to
// contain raw hex color literals — every other theme module imports tokens from
// here instead of inlining hex.
import { Platform } from "react-native";

// ---------------------------------------------------------------------------
// §1.1 Ember scale (brand accent ramp, shared by both themes)
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
  ember100: "#FFE7D3",
  ember200: "#FFC9A0",
  ember300: "#FFA96B",
  ember400: "#FF8A3D",
  ember500: "#F07A2E",
  ember600: "#C4601F",
  ember700: "#964916",
  ember900: "#45210A",
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
  /** Native approximation opacity for the top ambient ember wash. Machined retires the
   * thermal identity — always 0; kept so `Screen`'s old call sites still type-check. */
  forgeWashOpacity: number;
  /** Retired (Machined) — was HeatEdge's running-state gradient start. `HeatEdge` now
   * renders null; kept so the token still resolves for any lingering references. */
  heatEdgeFrom: string;
  /** Retired (Machined) — was HeatEdge's running-state gradient end. */
  heatEdgeTo: string;
  /** Retired (Machined) — was HeatEdge's outward glow shadow color. Zero-alpha. */
  heatGlow: string;
  /** Retired (Machined) — was StatusDot's busy-state radial halo. Zero-alpha; StatusDot
   * is now a flat dot with no halo. */
  dotGlow: string;
  /** Retired (Machined) — was Screen's single top ambient wash (web CSS radial-gradient
   * string). Zero-alpha; `Screen` no longer renders this. */
  forgeWash: string;
  /** De-boxed list row separator: a translucent hairline, NOT the solid `border` used on
   * card edges. */
  hairline: string;
  /** Retired (Machined) — was HeatEdge's "waiting" gradient start (a pending decision,
   * not just running). */
  waitingEdgeFrom: string;
  /** Retired (Machined) — was HeatEdge's "waiting" gradient end. */
  waitingEdgeTo: string;
  /** Retired (Machined) — was HeatEdge's "waiting" glow shadow color. Zero-alpha on both
   * themes now that the thermal edge is gone. */
  waitingGlow: string;
  /** Keyboard focus-visible ring (web) — low-alpha accent so tabbing reads as a quiet
   * hairline, never a solid box. */
  focusRing: string;
}

export const darkTokens: ColorTokens = {
  bg0: "#09090B",
  bg1: "#0D0D11",
  bg2: "#0E0E12",
  bg3: "#101015",
  borderStrong: "rgba(244,244,246,0.14)",
  border: "rgba(244,244,246,0.09)",
  ink: "#F4F4F6",
  ink2: "#9A9AA6",
  ink3: "#5F5F6B",
  ink4: "#45454F",
  accent: "#FF8A3D",
  accentPressed: "#F07A2E",
  onAccent: "#1A0E04",
  success: "#5FB97D",
  danger: "#E5605C",
  warn: "#D9A94E",
  info: "#7E9CB8",
  successBg: "#0F1D14",
  dangerBg: "#211012",
  warnBg: "#201808",
  warnBgInk: "#EFD9AC",
  selection: "rgba(255,138,61,0.14)",
  overlayScrim: "rgba(5,5,6,0.6)",
  accentTransparent: "rgba(255,138,61,0)",
  forgeWashOpacity: 0,
  ember: emberScale,
  heatEdgeFrom: emberScale.ember400,
  heatEdgeTo: emberScale.ember500,
  heatGlow: "rgba(255,138,61,0)",
  dotGlow: "rgba(255,138,61,0)",
  forgeWash: "radial-gradient(1100px 420px at 50% -8%, rgba(255,138,61,0), transparent 62%)",
  hairline: "rgba(244,244,246,0.07)",
  waitingEdgeFrom: "#E5605C",
  waitingEdgeTo: "#C24845",
  waitingGlow: "rgba(229,96,92,0)",
  focusRing: "rgba(255,138,61,0.4)",
};

export const lightTokens: ColorTokens = {
  bg0: "#F5F4F1",
  bg1: "#EFEDE8",
  bg2: "#FFFFFF",
  bg3: "#F7F6F3",
  borderStrong: "rgba(0,0,0,0.22)",
  border: "rgba(0,0,0,0.12)",
  ink: "#1C1B19",
  ink2: "#6E6A61",
  ink3: "#8A867D",
  ink4: "#B0ACA2",
  accent: "#D96A1E",
  accentPressed: "#C25C15",
  onAccent: "#FFFFFF",
  success: "#4C8A60",
  danger: "#C44A42",
  warn: "#9A7A2E",
  info: "#5B7C94",
  successBg: "#E6F0E8",
  dangerBg: "#F8E6E3",
  warnBg: "#F5EDD8",
  // §1.3 has no ink override for warnBg — the default `ink` already reads fine
  // on the paper-toned warnBg, unlike dark's near-black warnBg.
  warnBgInk: "#1C1B19",
  selection: "#F6E3D2",
  overlayScrim: "rgba(28,27,25,0.35)",
  accentTransparent: "rgba(217,106,30,0)",
  forgeWashOpacity: 0,
  ember: emberScale,
  heatEdgeFrom: emberScale.ember400,
  heatEdgeTo: emberScale.ember500,
  heatGlow: "rgba(217,106,30,0)",
  dotGlow: "rgba(217,106,30,0)",
  forgeWash: "radial-gradient(1100px 420px at 50% -8%, rgba(217,106,30,0), transparent 62%)",
  hairline: "rgba(0,0,0,0.09)",
  waitingEdgeFrom: "#C44A42",
  waitingEdgeTo: "#A63832",
  waitingGlow: "rgba(196,74,66,0)",
  focusRing: "rgba(217,106,30,0.4)",
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
  /** Segmented track (outer). Distinct from `radius8`/`radius12`: Segmented is the
   * one control with its own outer/inner pair, not the general button/card scale. */
  radiusSegmentOuter: number;
  /** Segmented thumb (inner). */
  radiusSegmentInner: number;
}

// Machined compresses the whole radius scale toward the 3-4px "machined" range —
// key names are historical (tied to the old pixel values) and no longer describe
// their own numbers; treat them as opaque scale steps, not px==name.
export const radii: RadiiScale = {
  radius4: 3,
  radius8: 4,
  radius12: 4,
  radius16: 6,
  radiusPill: 999,
  radiusSegmentOuter: 4,
  radiusSegmentInner: 2,
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
    shadowOpacity: 0.03,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
};

/** Exported for ds/ components that need a tinted-alpha variant of a semantic hex token
 * (e.g. Button's danger-tinted outline border) without hardcoding a raw color literal. */
export function hexToRgba(hex: string, alpha: number): string {
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
