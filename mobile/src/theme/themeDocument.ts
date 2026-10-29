import type { ColorTokens } from "./tokens";

export const PALETTE_KEYS = ["bg0", "bg1", "bg2", "bg3", "ink", "ink2", "ink3", "accent", "onAccent", "success", "danger", "warn"] as const;
export type PaletteKey = (typeof PALETTE_KEYS)[number];
export type ThemePalette = Record<PaletteKey, string>;

export interface ThemeDocument {
  id: string;
  name: string;
  light: ThemePalette;
  dark: ThemePalette;
}

export const THEME_IMPORT_LIMIT_BYTES = 32 * 1024;
const HEX = /^#[0-9a-fA-F]{6}$/;

function rgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

function toHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((c) => Math.round(Math.min(255, Math.max(0, c))).toString(16).padStart(2, "0")).join("").toUpperCase()}`;
}

export function mixHex(a: string, b: string, weightOfB: number): string {
  const [ar, ag, ab] = rgb(a);
  const [br, bg, bb] = rgb(b);
  return toHex([ar + (br - ar) * weightOfB, ag + (bg - ag) * weightOfB, ab + (bb - ab) * weightOfB]);
}

function hexToRgba(hex: string, alpha: number): string {
  const [r, g, b] = rgb(hex);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function luminance(hex: string): number {
  const [r, g, b] = rgb(hex).map((c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

export interface ContrastCheck {
  label: string;
  fg: PaletteKey;
  bg: PaletteKey;
  min: number;
  ratio: number;
  pass: boolean;
}

const CONTRAST_PAIRS: readonly [label: string, fg: PaletteKey, bg: PaletteKey, min: number][] = [
  ["Text on background", "ink", "bg0", 4.5],
  ["Text on cards", "ink", "bg2", 4.5],
  ["Secondary text", "ink2", "bg0", 4.5],
  ["Muted text", "ink3", "bg0", 3],
  ["Accent on background", "accent", "bg0", 3],
  ["Label on accent", "onAccent", "accent", 4.5],
];

export function contrastReport(palette: ThemePalette): ContrastCheck[] {
  return CONTRAST_PAIRS.map(([label, fg, bg, min]) => {
    const ratio = contrastRatio(palette[fg], palette[bg]);
    return { label, fg, bg, min, ratio, pass: ratio >= min };
  });
}

export function applyPalette(base: ColorTokens, p: ThemePalette, scheme: "light" | "dark"): ColorTokens {
  const light = scheme === "light";
  return {
    ...base,
    ...p,
    ink4: mixHex(p.ink3, p.bg0, 0.45),
    borderStrong: hexToRgba(p.ink, light ? 0.22 : 0.14),
    border: hexToRgba(p.ink, light ? 0.12 : 0.09),
    hairline: hexToRgba(p.ink, light ? 0.09 : 0.07),
    accentPressed: mixHex(p.accent, "#000000", 0.1),
    successBg: mixHex(p.bg0, p.success, light ? 0.14 : 0.1),
    dangerBg: mixHex(p.bg0, p.danger, light ? 0.14 : 0.1),
    warnBg: mixHex(p.bg0, p.warn, light ? 0.18 : 0.1),
    warnBgInk: light ? p.ink : mixHex(p.warn, p.ink, 0.6),
    selection: hexToRgba(p.accent, light ? 0.16 : 0.14),
    accentTransparent: hexToRgba(p.accent, 0),
    focusRing: hexToRgba(p.accent, 0.4),
    heatGlow: hexToRgba(p.accent, 0),
    dotGlow: hexToRgba(p.accent, 0),
    waitingEdgeFrom: p.danger,
    waitingEdgeTo: mixHex(p.danger, "#000000", 0.15),
  };
}

export function slugify(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40);
}

function readPalette(value: unknown, label: string): ThemePalette {
  if (typeof value !== "object" || value === null) throw new Error(`Theme is missing the ${label} palette`);
  const out = {} as ThemePalette;
  for (const key of PALETTE_KEYS) {
    const color = (value as Record<string, unknown>)[key];
    if (typeof color !== "string" || !HEX.test(color)) throw new Error(`${label}.${key} must be a #RRGGBB colour`);
    out[key] = color.toUpperCase();
  }
  return out;
}

export function parseThemeDocument(text: string): ThemeDocument {
  if (new TextEncoder().encode(text).length > THEME_IMPORT_LIMIT_BYTES) throw new Error("Theme file is larger than 32 KiB");
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new Error("Theme file is not valid JSON");
  }
  if (typeof raw !== "object" || raw === null) throw new Error("Theme file must be a JSON object");
  const obj = raw as Record<string, unknown>;
  const name = typeof obj.name === "string" ? obj.name.trim().slice(0, 40) : "";
  const slug = slugify(name || (typeof obj.id === "string" ? obj.id.replace(/^custom:/, "") : ""));
  if (!slug) throw new Error("Theme needs a name");
  return { id: `custom:${slug}`, name: name || slug, light: readPalette(obj.light, "light"), dark: readPalette(obj.dark, "dark") };
}

export function serializeThemeDocument(doc: ThemeDocument): string {
  return JSON.stringify({ id: doc.id, name: doc.name, light: doc.light, dark: doc.dark }, null, 2);
}
