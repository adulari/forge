// DESIGN_SYSTEM.md §6 Containers — Card is Machined's one elevated container:
// bg2, 1px border, radius 4. No shadow on either theme: Machined reads depth
// through hairlines, not elevation — only Sheet/overlay surfaces still carry
// `depth.sheet`. Machined retired the thermal (HeatEdge) identity entirely —
// a live/waiting card reads through its border tint + StatusDot, not a glow.
import React from "react";
import { StyleSheet, View, type ViewProps } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { cardPadding, radii } from "../../theme/tokens";

export interface CardProps extends ViewProps {
  /** Both variants render at Machined's single card radius (4) — kept for source compat
   * with existing call sites that still pass "feature" for plan/diff/permission cards. */
  variant?: "default" | "feature";
  /** Set false to opt out of the default 12x14 card padding (§3) for custom internal layout. */
  padded?: boolean;
}

export function Card({ variant = "default", padded = true, style, children, ...rest }: CardProps) {
  void variant; // both variants resolve to radius4 under Machined; see the prop doc above.
  const tokens = useTokens();
  return (
    <View
      style={[
        styles.base,
        {
          backgroundColor: tokens.bg2,
          borderColor: tokens.border,
          borderRadius: radii.radius4,
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
