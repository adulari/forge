// DESIGN_SYSTEM.md §2 `section` style: 10/14 600 +1 letter-spacing UPPERCASE,
// rendered in mono (Machined: section labels are technical text — same box
// metrics as `type.section`, family overridden to Geist Mono) in ink3 — used to
// head grouped rows (settings, palette results, gallery sections). Dropped the
// ember tick + trailing hairline rule the previous build carried over from
// DESIGN_ELEVATION.md Move 3 — section headers are a plain label; grouped rows
// below already carry their own hairline separators, and the header's own top
// padding (not a rule) is what separates one group from the next.
import React from "react";
import { StyleSheet, Text, type TextStyle, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export interface SectionHeaderProps {
  children: string;
  style?: TextStyle;
}

export function SectionHeader({ children, style }: SectionHeaderProps) {
  const tokens = useTokens();

  return (
    <View style={styles.wrap} accessibilityRole="header">
      <Text style={[typeScale.section, styles.mono, { color: tokens.ink3 }, style]} numberOfLines={1}>
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
  mono: { fontFamily: monoFamily.regular },
});
