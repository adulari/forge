// DESIGN_SYSTEM.md §2 `section` style: 11/14 700 +0.6 letter-spacing UPPERCASE
// ink3 — used to head grouped rows (settings, palette results, gallery sections).
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
      <Text style={[typeScale.section, { color: tokens.ink3 }, style]} numberOfLines={1}>
        {children}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    paddingHorizontal: space.space16,
    paddingTop: space.space16,
    paddingBottom: space.space8,
  },
});
