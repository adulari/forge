// DESIGN_SYSTEM.md §2 (Typography), verbatim scale.
import { Platform, type TextStyle } from "react-native";

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

// §2: "Sans: platform system stack (SF / Roboto / system-ui). No custom sans." On
// native, leaving fontFamily unset already renders the platform system font. On web,
// react-native-web's own Text default falls back to the invalid CSS family "System"
// (not the `system-ui` keyword), which browsers can't match — hence the serif fallback
// this constant fixes. Native is left untouched (undefined = no-op style key).
const sansFamily =
  Platform.OS === "web" ? "system-ui, -apple-system, BlinkMacSystemFont, sans-serif" : undefined;

// ---------------------------------------------------------------------------
// §2 type scale — size / line-height / weight, the only allowed combinations.
// ---------------------------------------------------------------------------

export const type = {
  display: { fontSize: 28, lineHeight: 34, fontWeight: "700", fontFamily: sansFamily } satisfies TextStyle,
  title: {
    fontSize: 20,
    lineHeight: 26,
    fontWeight: "700",
    letterSpacing: -0.4,
    fontFamily: sansFamily,
  } satisfies TextStyle,
  // Row title (session/list row names) — 600, no letterSpacing. Screen headers use
  // `headingBold` below instead; both share the 17/24 box so they line up when adjacent.
  heading: { fontSize: 17, lineHeight: 24, fontWeight: "600", fontFamily: sansFamily } satisfies TextStyle,
  // Hearth screen-header variant of `heading` (e.g. the Session Chat title) — 700,
  // letterSpacing -0.3. Kept as its own key so existing `type.heading` row-title call
  // sites are untouched.
  headingBold: {
    fontSize: 17,
    lineHeight: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
    fontFamily: sansFamily,
  } satisfies TextStyle,
  body: { fontSize: 15, lineHeight: 22, fontWeight: "400", fontFamily: sansFamily } satisfies TextStyle,
  bodyBold: { fontSize: 15, lineHeight: 22, fontWeight: "600", fontFamily: sansFamily } satisfies TextStyle,
  sub: { fontSize: 13, lineHeight: 18, fontWeight: "400", fontFamily: sansFamily } satisfies TextStyle,
  meta: { fontSize: 12, lineHeight: 16, fontWeight: "500", fontFamily: sansFamily } satisfies TextStyle,
  // Hearth: section labels move from ink3 to ink4 (color lives in tokens.ts —
  // ColorTokens.ink4 — consumers merge `type.section` with `{ color: tokens.ink4 }`;
  // `SectionHeader.tsx` is the reference call site). letterSpacing widened 0.6 -> 0.8.
  section: {
    fontSize: 11,
    lineHeight: 14,
    fontWeight: "700",
    letterSpacing: 0.8,
    textTransform: "uppercase",
    fontFamily: sansFamily,
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
  // Hearth mono discipline: numbers/paths/branches/model ids/commands render in
  // JetBrains Mono at 11-12px with tabular-nums (never proportional sans). `codeSmall`
  // (12px) covers most meta rows (cost, ctx%, model id beside a title); `monoMeta` (11px)
  // is the tightest tier — secondary figures beside a bigger number (e.g. a relative
  // timestamp under a cost). Always pair with `tabularNums` from this file.
  monoMeta: {
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
} as const;

export type TypeToken = keyof typeof type;

// iOS Safari auto-zooms the page when focusing a text input rendered below 16px —
// `type.body` is 15px, below that threshold. Web-only bump applied on top of
// `type.body` at the two TextInput call sites (Input.tsx, Composer.tsx); native
// sizes are untouched (empty object, no fontSize key, on iOS/Android).
export const webInputTextStyle: TextStyle = Platform.OS === "web" ? { fontSize: 16 } : {};

// ---------------------------------------------------------------------------
// Format helpers (§2)
// ---------------------------------------------------------------------------

/** `$0.0421` (4dp) under $1, `$12.48` (2dp) at/above $1. */
export function formatCost(usd: number): string {
  const magnitude = Math.abs(usd);
  return magnitude < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
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

/**
 * Friendly cwd label for fleet cards / session header. Detects the Forge worktree
 * pattern `<repo>/.forge/worktrees/<hash>` and collapses it to `repo · wt <short>`
 * (first 8 chars of the hash). Otherwise returns the basename of the path.
 * The full path stays available via the `accessibilityLabel` / `title` prop at
 * the call site.
 */
export function formatCwd(cwd: string): string {
  const wtMatch = cwd.match(/^(.+?)\/\.forge\/worktrees\/([a-f0-9-]+)/i);
  if (wtMatch) {
    const repo = wtMatch[1].replace(/\/+$/, "").split("/").pop() ?? wtMatch[1];
    const shortHash = wtMatch[2].slice(0, 8);
    return `${repo} · wt ${shortHash}`;
  }
  // Fall back to basename for non-worktree paths.
  const parts = cwd.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || cwd;
}
