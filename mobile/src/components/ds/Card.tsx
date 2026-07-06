// DESIGN_SYSTEM.md §6 Containers — Card: bg2, radius 12, hairline; `feature`
// variant radius 16 (plan/diff/permission cards).
import React from "react";
import { StyleSheet, View, type ViewProps } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { cardPadding, radii } from "../../theme/tokens";

export interface CardProps extends ViewProps {
  variant?: "default" | "feature";
  /** Set false to opt out of the default 12x14 card padding (§3) for custom internal layout. */
  padded?: boolean;
}

export function Card({ variant = "default", padded = true, style, children, ...rest }: CardProps) {
  const tokens = useTokens();
  return (
    <View
      style={[
        styles.base,
        {
          backgroundColor: tokens.bg2,
          borderColor: tokens.border,
          borderRadius: variant === "feature" ? radii.radius16 : radii.radius12,
        },
        padded && styles.padded,
        style,
      ]}
      {...rest}
    >
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  base: { borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  padded: { paddingHorizontal: cardPadding.x, paddingVertical: cardPadding.y },
});
