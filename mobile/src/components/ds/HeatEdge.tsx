// Thermal state edge for live/waiting Emberline surfaces. Hearth core rule 3: the
// bar itself is static (the prototype's heat edges carry no animation) — StatusDot
// owns all state-liveness pulsing.
import React from "react";
import { StyleSheet, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";

import { useTokens } from "../../theme/ThemeProvider";
import { shadowStyle } from "../../theme/tokens";

const WIDTH = 2;
const GLOW_RADIUS = 12;

export interface HeatEdgeProps {
  state?: "busy" | "waiting" | false;
  /** @deprecated legacy boolean API — `active` maps to `state="busy"`. Kept so existing
   *  call sites (composer heat edge, floor tiles) compile against the new state API. */
  active?: boolean;
}

export function HeatEdge({ state, active }: HeatEdgeProps) {
  const tokens = useTokens();
  const resolved: "busy" | "waiting" | false = state ?? (active ? "busy" : false);
  if (!resolved) return null;

  // Hearth core rule 3: running = ember gradient/glow, waiting = the danger gradient/glow
  // — never swapped, and each theme (dark/light) supplies its own pair (see tokens.ts).
  const [from, to, glow] =
    resolved === "waiting"
      ? [tokens.waitingEdgeFrom, tokens.waitingEdgeTo, tokens.waitingGlow]
      : [tokens.heatEdgeFrom, tokens.heatEdgeTo, tokens.heatGlow];

  return (
    <View style={styles.wrap} pointerEvents="none">
      <LinearGradient
        colors={[from, to]}
        start={{ x: 0, y: 0 }}
        end={{ x: 0, y: 1 }}
        style={[
          styles.bar,
          shadowStyle({
            shadowColor: glow,
            shadowOpacity: 1,
            shadowRadius: GLOW_RADIUS,
            shadowOffset: { width: 0, height: 0 },
            elevation: 0,
          }),
        ]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "absolute", left: 0, top: 0, bottom: 0, width: WIDTH },
  bar: { width: WIDTH, height: "100%" },
});
