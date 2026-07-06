// DESIGN_SYSTEM.md §2 (Typography), verbatim scale.
import type { TextStyle } from "react-native";

/**
 * Bundled mono font (JetBrains Mono). Native embedding (expo-font config plugin)
 * requires the exact PostScript name per weight — RN cannot vary a custom font's
 * weight via `fontWeight`, so callers pick the family per weight instead.
 */
export const monoFamily = {
  regular: "JetBrainsMono-Regular",
  bold: "JetBrainsMono-Bold",
} as const;

export function monoFamilyFor(weight: 400 | 700): string {
  return weight === 700 ? monoFamily.bold : monoFamily.regular;
}

export const tabularNums: TextStyle = { fontVariant: ["tabular-nums"] };

// ---------------------------------------------------------------------------
// §2 type scale — size / line-height / weight, the only allowed combinations.
// ---------------------------------------------------------------------------

export const type = {
  display: { fontSize: 28, lineHeight: 34, fontWeight: "700" } satisfies TextStyle,
  title: { fontSize: 20, lineHeight: 26, fontWeight: "700" } satisfies TextStyle,
  heading: { fontSize: 17, lineHeight: 24, fontWeight: "600" } satisfies TextStyle,
  body: { fontSize: 15, lineHeight: 22, fontWeight: "400" } satisfies TextStyle,
  bodyBold: { fontSize: 15, lineHeight: 22, fontWeight: "600" } satisfies TextStyle,
  sub: { fontSize: 13, lineHeight: 18, fontWeight: "400" } satisfies TextStyle,
  meta: { fontSize: 12, lineHeight: 16, fontWeight: "500" } satisfies TextStyle,
  // §1.4/§2: section headers are also ink3 + uppercase — color lives in tokens.ts
  // (ColorTokens.ink3), so consumers merge `type.section` with `{ color: tokens.ink3 }`.
  section: {
    fontSize: 11,
    lineHeight: 14,
    fontWeight: "700",
    letterSpacing: 0.6,
    textTransform: "uppercase",
  } satisfies TextStyle,
  code: {
    fontSize: 13,
    lineHeight: 20,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
  codeSmall: {
    fontSize: 12,
    lineHeight: 18,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
} as const;

export type TypeToken = keyof typeof type;

// ---------------------------------------------------------------------------
// Format helpers (§2)
// ---------------------------------------------------------------------------

/** `$0.0421` (4dp) under $1, `$12.48` (2dp) at/above $1. */
export function formatCost(usd: number): string {
  return Math.abs(usd) < 1 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

function formatTokenCount(n: number): string {
  if (n < 1000) return `${Math.round(n)}`;
  const rounded = Math.round((n / 1000) * 10) / 10;
  return Number.isInteger(rounded) ? `${rounded}k` : `${rounded.toFixed(1)}k`;
}

/** `128.4k / 200k` */
export function formatTokenPair(used: number, total: number): string {
  return `${formatTokenCount(used)} / ${formatTokenCount(total)}`;
}

/** `12s` · `4m` · `2h` · `3d` — single-tier relative time, coarsest applicable unit. */
export function formatRelativeTime(fromMs: number, nowMs: number = Date.now()): string {
  const deltaSec = Math.max(0, Math.round((nowMs - fromMs) / 1000));
  if (deltaSec < 60) return `${deltaSec}s`;
  const deltaMin = Math.round(deltaSec / 60);
  if (deltaMin < 60) return `${deltaMin}m`;
  const deltaHour = Math.round(deltaMin / 60);
  if (deltaHour < 24) return `${deltaHour}h`;
  const deltaDay = Math.round(deltaHour / 24);
  return `${deltaDay}d`;
}
