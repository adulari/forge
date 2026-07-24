// mobile/redesign/DESIGN_SYSTEM.md §2 (Typography), verbatim scale. Machined —
// supersedes Emberline: Geist (UI) + Geist Mono (technical/status/diff text)
// everywhere, replacing the prior system-sans + JetBrains Mono pairing.
import { Platform, type TextStyle } from "react-native";

/**
 * Bundled mono font (Geist Mono). Native embedding (expo-font config plugin)
 * requires the exact PostScript name per weight — RN cannot vary a custom font's
 * weight via `fontWeight`, so callers pick the family per weight instead. The
 * design never uses mono at 700 — `bold` maps to SemiBold (600), not a true bold
 * cut; the key name stays `bold` so existing call sites keep compiling.
 */
export const monoFamily = {
  regular: "GeistMono-Regular",
  medium: "GeistMono-Medium",
  bold: "GeistMono-SemiBold",
} as const;

export function monoFamilyFor(weight: 400 | 700): string {
  return weight === 700 ? monoFamily.bold : monoFamily.regular;
}

export const tabularNums: TextStyle = { fontVariant: ["tabular-nums"] };

// §2: sans is now a bundled custom family (Geist) on every platform — native embeds
// it via the expo-font config plugin (same mechanism as the mono family below);
// web ships it as its own @font-face (+html.tsx) with Inter/system-ui as fallbacks
// for the brief pre-load flash and any font-loading failure. RN cannot vary a
// custom family's weight via `fontWeight` on native, so every per-weight `type`
// token below resolves its family through `sansFamilyFor` instead of a single
// constant.
export function sansFamilyFor(weight: 400 | 500 | 600 | 700): string {
  if (Platform.OS === "web") {
    return "Geist, Inter, system-ui, sans-serif";
  }
  switch (weight) {
    case 700:
      return "Geist-Bold";
    case 600:
      return "Geist-SemiBold";
    case 500:
      return "Geist-Medium";
    case 400:
    default:
      return "Geist-Regular";
  }
}

// ---------------------------------------------------------------------------
// §2 type scale — size / line-height / weight, the only allowed combinations.
// "2a Air" recalibration: mono floor 10.5px, dense chrome + generous chat prose.
// ---------------------------------------------------------------------------

export const type = {
  display: {
    fontSize: 26,
    lineHeight: 32,
    fontWeight: "700",
    letterSpacing: -0.5,
    fontFamily: sansFamilyFor(700),
  } satisfies TextStyle,
  title: {
    fontSize: 18,
    lineHeight: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
    fontFamily: sansFamilyFor(700),
  } satisfies TextStyle,
  // Row title (session/list row names) — 600, no letterSpacing. Screen headers use
  // `headingBold` below instead; both share the 15/22 box so they line up when adjacent.
  heading: {
    fontSize: 15,
    lineHeight: 22,
    fontWeight: "600",
    fontFamily: sansFamilyFor(600),
  } satisfies TextStyle,
  // Screen-header variant of `heading` (e.g. the Session Chat title) — 700,
  // letterSpacing -0.2. Kept as its own key so existing `type.heading` row-title call
  // sites are untouched.
  headingBold: {
    fontSize: 15,
    lineHeight: 22,
    fontWeight: "700",
    letterSpacing: -0.2,
    fontFamily: sansFamilyFor(700),
  } satisfies TextStyle,
  body: {
    fontSize: 13,
    lineHeight: 21,
    fontWeight: "400",
    fontFamily: sansFamilyFor(400),
  } satisfies TextStyle,
  bodyBold: {
    fontSize: 13,
    lineHeight: 21,
    fontWeight: "600",
    fontFamily: sansFamilyFor(600),
  } satisfies TextStyle,
  sub: {
    fontSize: 12,
    lineHeight: 17,
    fontWeight: "400",
    fontFamily: sansFamilyFor(400),
  } satisfies TextStyle,
  meta: {
    fontSize: 11,
    lineHeight: 15,
    fontWeight: "500",
    fontFamily: sansFamilyFor(500),
  } satisfies TextStyle,
  // Section labels move color from ink3 to ink4 (color lives in tokens.ts —
  // ColorTokens.ink4 — consumers merge `type.section` with `{ color: tokens.ink4 }`).
  // This is the sans-weight base box; `SectionHeader.tsx` layers a mono `fontFamily`
  // override on top per the ds component rules (section labels are "technical text").
  section: {
    fontSize: 10,
    lineHeight: 14,
    fontWeight: "600",
    letterSpacing: 1,
    textTransform: "uppercase",
    fontFamily: sansFamilyFor(600),
  } satisfies TextStyle,
  code: {
    fontSize: 12,
    lineHeight: 18,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
  codeSmall: {
    fontSize: 11,
    lineHeight: 16,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
  // Mono discipline: numbers/paths/branches/model ids/commands render in Geist Mono
  // at 10.5-12px with tabular-nums (never proportional sans). `codeSmall` (11px)
  // covers most meta rows (cost, ctx%, model id beside a title); `monoMeta` (10.5px)
  // is the tightest tier — secondary figures beside a bigger number (e.g. a relative
  // timestamp under a cost). Always pair with `tabularNums` from this file.
  monoMeta: {
    fontSize: 10.5,
    lineHeight: 14,
    fontWeight: "400",
    fontFamily: monoFamily.regular,
  } satisfies TextStyle,
} as const;

export type TypeToken = keyof typeof type;

// iOS Safari auto-zooms the page when focusing a text input rendered below 16px —
// `type.body` is 13px, well below that threshold. Web-only bump applied on top of
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
