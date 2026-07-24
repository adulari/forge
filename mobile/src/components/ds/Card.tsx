// DESIGN_SYSTEM.md §6 Containers — Card is Machined's one elevated container:
// bg2, 1px border, radius 4, optional 2px left HeatEdge slot (visually inert —
// Machined retired the thermal edge, see HeatEdge.tsx). No shadow on either
// theme: Machined reads depth through hairlines, not elevation — only Sheet/
// overlay surfaces still carry `depth.sheet`.
import React from "react";
import { StyleSheet, View, type ViewProps } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { cardPadding, radii } from "../../theme/tokens";
import { HeatEdge, type HeatEdgeProps } from "./HeatEdge";

export interface CardProps extends ViewProps {
  /** Both variants render at Machined's single card radius (4) — kept for source compat
   * with existing call sites that still pass "feature" for plan/diff/permission cards. */
  variant?: "default" | "feature";
  /** Set false to opt out of the default 12x14 card padding (§3) for custom internal layout. */
  padded?: boolean;
  /** Left HeatEdge — omit (or false) for an idle card, "busy"/"waiting" for a live one.
   * Renders nothing under Machined (thermal identity retired); kept for source compat. */
  heatEdge?: HeatEdgeProps["state"];
}

export function Card({ variant = "default", padded = true, heatEdge = false, style, children, ...rest }: CardProps) {
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
        heatEdge && styles.heatEdgeInset,
        style,
      ]}
      {...rest}
    >
      {heatEdge ? <HeatEdge state={heatEdge} /> : null}
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  base: { borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  padded: { paddingHorizontal: cardPadding.x, paddingVertical: cardPadding.y },
  heatEdgeInset: { paddingLeft: cardPadding.x + 2 },
});
