// DESIGN_ELEVATION.md Move 1 (thermal identity) — HeatEdge: a live/working
// session's "state is temperature" signature. A 3px leading-left vertical bar,
// gradient `heatEdgeFrom -> heatEdgeTo`, plus a soft outward ember glow.
// Idle/done sessions render nothing (cool). Static — no animation to reduce.
//
// Consumed by SessionCard (EW2) as the de-boxed row's only container-ish
// affordance, bleeding to the screen edge (Move 2). Usage:
//
//   <View style={{ position: "relative" }}>
//     <HeatEdge active={isLive} />
//     ...row content...
//   </View>
//
// Position the wrapping row with `position: "relative"` and let HeatEdge fill
// it edge-to-edge (top/bottom 0) — it does not reserve layout space itself.
import React from "react";
import { StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";

import { useTokens } from "../../theme/ThemeProvider";

const WIDTH = 3;
const GLOW_RADIUS = 16;

export interface HeatEdgeProps {
  /** Renders nothing when false/omitted — idle/done sessions have no heat edge. */
  active?: boolean;
}

export function HeatEdge({ active = false }: HeatEdgeProps) {
  const tokens = useTokens();

  if (!active) return null;

  return (
    <LinearGradient
      colors={[tokens.heatEdgeFrom, tokens.heatEdgeTo]}
      start={{ x: 0, y: 0 }}
      end={{ x: 0, y: 1 }}
      style={[
        styles.bar,
        {
          pointerEvents: "none",
          shadowColor: tokens.heatGlow,
          shadowOpacity: 1,
          shadowRadius: GLOW_RADIUS,
          shadowOffset: { width: 0, height: 0 },
        },
      ]}
    />
  );
}

const styles = StyleSheet.create({
  bar: {
    position: "absolute",
    left: 0,
    top: 0,
    bottom: 0,
    width: WIDTH,
  },
});
