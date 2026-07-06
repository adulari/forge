// Design tokens — verbatim from BUILD_PLAN.md §4 (source: remote_assets/styles.css).
// Raw values (colors/radii/spacing) live in ./tokens.json — the single source of truth
// shared with tailwind.config.js (which requires that JSON directly, since it runs as
// plain Node/CommonJS outside the TS toolchain). This file re-exports them typed and
// layers on non-Tailwind metadata (typography, motion). It is the ONLY .ts file allowed
// to contain raw hex color literals (UI_RULES.md #9) — tokens.json is its raw twin.
// Dark-only app: no light theme.
import rawTokens from "./tokens.json";

export const colors = rawTokens.colors;

export type ColorToken = keyof typeof colors;

// Radii scale (BUILD_PLAN §4)
export const radii = rawTokens.radii;

// Spacing scale (UI_RULES.md #8) — the ONLY spacing values allowed anywhere.
export const spacing = rawTokens.spacing;

export const screenGutter = spacing["12"];
export const cardPaddingX = spacing["10"];
export const cardPaddingY = spacing["8"];
export const buttonPaddingX = 16;
export const buttonPaddingY = 11;
export const minTapTarget = 44;

// Typography (BUILD_PLAN §4 + UI_RULES #33)
export const fontSizes = {
  body: 15, // base body text
  transcript: 14,
  meta: 12, // status/meta
  metaLg: 13,
  mono: 12, // code/diff mono
  h1: 16,
  sectionHead: 11,
  sectionHeadLg: 13,
} as const;

export const lineHeights = {
  body: 1.5 * fontSizes.body,
  mono: 1.5 * fontSizes.mono,
} as const;

export const fontWeights = {
  regular: "400" as const,
  semibold: "600" as const,
  bold: "700" as const,
};

// System font stack — web UI uses -apple-system/system-ui; load NO custom font.
export const fontFamily = {
  sans: undefined, // platform default (San Francisco on iOS)
  mono: "ui-monospace" as const, // falls back to Menlo via monospace.ts helper
};

// Pulse animation durations (ms) — busy vs waiting dot pulse (UI_RULES #18, #28)
export const pulse = {
  busyDurationMs: 1000,
  waitingDurationMs: 700,
  minOpacity: 0.35,
};

export const motionDurationMs = 200; // max entrance/transition duration (UI_RULES #18, #29)

export const theme = {
  colors,
  radii,
  spacing,
  screenGutter,
  cardPaddingX,
  cardPaddingY,
  buttonPaddingX,
  buttonPaddingY,
  minTapTarget,
  fontSizes,
  lineHeights,
  fontWeights,
  fontFamily,
  pulse,
  motionDurationMs,
} as const;

export type Theme = typeof theme;
export default theme;
