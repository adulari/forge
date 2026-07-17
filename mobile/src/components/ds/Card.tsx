// DESIGN_SYSTEM.md §6 Containers — Card is Hearth's one elevated container, the
// "decision card" (core rule 2): bg2, 1px border, radius 16, optional 2px left
// HeatEdge. Dark theme carries no shadow (hairlines only); light theme gets the
// paper card shadow (`depthLight.raised`, already spec'd to HANDOFF's "card shadow").
import React from "react";
import { StyleSheet, View, type ViewProps } from "react-native";

import { useTheme } from "../../theme/ThemeProvider";
import { cardPadding, depthDark, depthLight, radii, shadowStyle } from "../../theme/tokens";
import { HeatEdge, type HeatEdgeProps } from "./HeatEdge";

export interface CardProps extends ViewProps {
  /** Both variants render at Hearth's single card radius (16) — kept for source compat
   * with existing call sites that still pass "feature" for plan/diff/permission cards. */
  variant?: "default" | "feature";
  /** Set false to opt out of the default 12x14 card padding (§3) for custom internal layout. */
  padded?: boolean;
  /** Left HeatEdge — omit (or false) for an idle card, "busy"/"waiting" for a live one. */
  heatEdge?: HeatEdgeProps["state"];
}

export function Card({ variant = "default", padded = true, heatEdge = false, style, children, ...rest }: CardProps) {
  void variant; // both variants resolve to radius16 under Hearth; see the prop doc above.
  const { scheme, tokens } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  return (
    <View
      style={[
        styles.base,
        {
          backgroundColor: tokens.bg2,
          borderColor: tokens.border,
          borderRadius: radii.radius16,
        },
        depth.raised ? shadowStyle(depth.raised) : undefined,
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
