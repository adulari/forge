// DESIGN_SYSTEM.md §2 `section` style: 11/14 700 +0.8 letter-spacing UPPERCASE
// ink4 (Hearth: moved from ink3) — used to head grouped rows (settings, palette
// results, gallery sections). Hearth fidelity fix: dropped the ember tick +
// trailing hairline rule the previous build carried over from DESIGN_ELEVATION.md
// Move 3 — the redesign's section headers are a plain label; grouped rows below
// already carry their own hairline separators, and the header's own top padding
// (not a rule) is what separates one group from the next.
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
      <Text style={[typeScale.section, { color: tokens.ink4 }, style]} numberOfLines={1}>
        {children}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    paddingHorizontal: space.space16,
    paddingTop: space.space12,
    paddingBottom: space.space4,
  },
});
