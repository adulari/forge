// DESIGN_SYSTEM.md §2 `section` style: 11/14 700 +0.8 letter-spacing UPPERCASE
// ink4 (Hearth: moved from ink3) — used to head grouped rows (settings, palette
// results, gallery sections). DESIGN_ELEVATION.md Move 3 — a 6px ember tick
// precedes the label, then a hairline rule fills the rest of the row.
import React from "react";
import { StyleSheet, Text, type TextStyle, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface SectionHeaderProps {
  children: string;
  style?: TextStyle;
}

export function SectionHeader({ children, style }: SectionHeaderProps) {
  const tokens = useTokens();

  return (
    <View style={styles.wrap} accessibilityRole="header">
      <View style={[styles.tick, { backgroundColor: tokens.accent }]} />
      <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }, style]} numberOfLines={1}>
        {children}
      </Text>
      <View style={[styles.rule, { backgroundColor: tokens.border }]} />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space16,
    paddingTop: space.space16,
    paddingBottom: space.space8,
  },
  tick: {
    width: 6,
    height: 2,
  },
  label: {
    flexShrink: 1,
    marginLeft: space.space8,
    marginRight: space.space8,
  },
  rule: {
    flex: 1,
    height: StyleSheet.hairlineWidth,
  },
});
