// DESIGN_SYSTEM.md §6 Status & data: `Badge` — tones neutral/accent/success/danger/
// warn/outline; 4-radius `small` or `pill` shape. Tone backgrounds reuse the existing
// semantic *Bg tokens (§1.2/§1.3) rather than inventing new hex — `warn` uses
// `warnBgInk` since that token exists precisely to pick readable text on `warnBg`
// across both themes; `accent` reuses the ember-tinted `selection` well.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type ColorTokens } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export type BadgeTone = "neutral" | "accent" | "success" | "danger" | "warn" | "outline";
export type BadgeShape = "small" | "pill";

export interface BadgeProps {
  label: string;
  tone?: BadgeTone;
  shape?: BadgeShape; // optional: defaults to pill for status tones, small for descriptive
}

interface ToneStyle {
  background: string;
  ink: string;
  borderColor?: string;
}

function toneStyle(tone: BadgeTone, tokens: ColorTokens): ToneStyle {
  switch (tone) {
    case "accent":
      return { background: tokens.selection, ink: tokens.accent };
    case "success":
      return { background: tokens.successBg, ink: tokens.success };
    case "danger":
      return { background: tokens.dangerBg, ink: tokens.danger };
    case "warn":
      return { background: tokens.warnBg, ink: tokens.warnBgInk };
    case "outline":
      return { background: "transparent", ink: tokens.ink2, borderColor: tokens.border };
    case "neutral":
    default:
      return { background: tokens.bg3, ink: tokens.ink2 };
  }
}

export function Badge({ label, tone = "neutral", shape: shapeProp }: BadgeProps) {
  const tokens = useTokens();
  const { background, ink, borderColor } = toneStyle(tone, tokens);

  // Drive casing and shape from tone/role per DESIGN_SYSTEM.md §6
  // Status badges (danger/accent/success/warn) → uppercase + pill
  // Descriptive badges (neutral/outline) → sentence-case + small
  const isStatusTone = tone === "danger" || tone === "accent" || tone === "success" || tone === "warn";
  const shape = shapeProp ?? (isStatusTone ? "pill" : "small");
  const textTransform = isStatusTone ? "uppercase" : "none";

  return (
    <View
      style={[
        styles.base,
        shape === "pill" ? styles.pill : styles.small,
        { backgroundColor: background },
        borderColor ? { borderWidth: StyleSheet.hairlineWidth, borderColor } : null,
      ]}
      accessibilityRole="text"
      accessibilityLabel={label}
    >
      <Text style={[typeScale.meta, { color: ink, textTransform }]} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  base: {
    alignSelf: "flex-start",
    paddingHorizontal: space.space8,
    paddingVertical: 2,
  },
  small: { borderRadius: radii.radius4 },
  pill: { borderRadius: radii.radiusPill },
});
