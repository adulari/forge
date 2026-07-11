// Thermal state edge for live/waiting Emberline surfaces.
import React from "react";
import { StyleSheet } from "react-native";
import Animated from "react-native-reanimated";
import { LinearGradient } from "expo-linear-gradient";

import { useThermal } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { shadowStyle } from "../../theme/tokens";

const WIDTH = 3;
const GLOW_RADIUS = 16;

export interface HeatEdgeProps {
  state?: "busy" | "waiting" | false;
  /** @deprecated legacy boolean API — `active` maps to `state="busy"`. Kept so existing
   *  call sites (composer heat edge, floor tiles) compile against the new state API. */
  active?: boolean;
}

export function HeatEdge({ state, active }: HeatEdgeProps) {
  const tokens = useTokens();
  const resolved: "busy" | "waiting" | false = state ?? (active ? "busy" : false);
  const thermalStyle = useThermal(resolved || "off");
  if (!resolved) return null;

  return (
    <Animated.View style={[styles.wrap, thermalStyle]} pointerEvents="none">
      <LinearGradient
        colors={[tokens.heatEdgeFrom, tokens.heatEdgeTo]}
        start={{ x: 0, y: 0 }}
        end={{ x: 0, y: 1 }}
        style={[
          styles.bar,
          shadowStyle({
            shadowColor: tokens.heatGlow,
            shadowOpacity: 1,
            shadowRadius: GLOW_RADIUS,
            shadowOffset: { width: 0, height: 0 },
            elevation: 0,
          }),
        ]}
      />
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "absolute", left: 0, top: 0, bottom: 0, width: WIDTH },
  bar: { width: WIDTH, height: "100%" },
});
