import { describe, expect, it } from "vitest";

import { applyPalette, contrastRatio, contrastReport, parseThemeDocument, serializeThemeDocument, THEME_IMPORT_LIMIT_BYTES } from "./themeDocument";
import { THEME_PRESETS } from "./themePresets";
import type { ColorTokens } from "./tokens";

describe("theme documents", () => {
  it("computes WCAG ratios", () => {
    expect(contrastRatio("#000000", "#FFFFFF")).toBeCloseTo(21, 0);
    expect(contrastRatio("#777777", "#777777")).toBe(1);
  });

  it("ships presets that pass their own contrast checks", () => {
    expect(THEME_PRESETS.length).toBeGreaterThanOrEqual(6);
    for (const preset of THEME_PRESETS) {
      for (const scheme of ["light", "dark"] as const) {
        const failing = contrastReport(preset[scheme]).filter((check) => !check.pass).map((check) => `${preset.id}.${scheme} ${check.label} ${check.ratio.toFixed(2)}`);
        expect(failing).toEqual([]);
      }
    }
  });

  it("flags a low-contrast pair", () => {
    const palette = { ...THEME_PRESETS[0].dark, ink: "#20263A" };
    const failing = contrastReport(palette).filter((check) => !check.pass).map((check) => check.label);
    expect(failing).toContain("Text on background");
  });

  it("round-trips an export through import under a custom id", () => {
    const text = serializeThemeDocument({ ...THEME_PRESETS[1], name: "My Graphite!" });
    const doc = parseThemeDocument(text);
    expect(doc.id).toBe("custom:my-graphite");
    expect(doc.dark).toEqual(THEME_PRESETS[1].dark);
  });

  it("rejects oversize, malformed and non-hex input", () => {
    expect(() => parseThemeDocument(" ".repeat(THEME_IMPORT_LIMIT_BYTES + 1))).toThrow(/32 KiB/);
    expect(() => parseThemeDocument("{")).toThrow(/JSON/);
    const bad = JSON.parse(serializeThemeDocument(THEME_PRESETS[0]));
    bad.dark.accent = "orange";
    expect(() => parseThemeDocument(JSON.stringify(bad))).toThrow(/dark\.accent/);
  });

  it("derives a complete token set", () => {
    const tokens = applyPalette({ ember: {} } as unknown as ColorTokens, THEME_PRESETS[0].dark, "dark");
    expect(tokens.bg0).toBe(THEME_PRESETS[0].dark.bg0);
    expect(tokens.ink4).toMatch(/^#[0-9A-F]{6}$/);
    expect(tokens.focusRing).toMatch(/^rgba/);
  });
});
