// DESIGN_SYSTEM.md §6 Status & data: `ContextGauge` — 3px track (border color) +
// fill animated via Gaugeflow (§5.2), color steps accent -> warn (>70%) -> danger
// (>90%) per §1.4, `128.4k/200k` meta beside via `formatTokenPair`.
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useGaugeflow } from "../../theme/motion";
import { gaugeColor, radii, shadowStyle, space } from "../../theme/tokens";
import { formatTokenPair, tabularNums, type as typeScale } from "../../theme/typography";

export interface ContextGaugeProps {
  used: number;
  total: number;
}

const TRACK_HEIGHT = 3;

export function ContextGauge({ used, total }: ContextGaugeProps) {
  const tokens = useTokens();
  const pct = total > 0 ? (used / total) * 100 : 0;
  const clampedPct = Math.max(0, Math.min(100, pct));
  const { style: fillStyle } = useGaugeflow(pct);
  const fillColor = gaugeColor(pct, tokens);
  // DESIGN_ELEVATION.md Move 1 — "overheat": faint same-color glow once the
  // gauge crosses into warn (>70%) / danger (>90%). Below 70%: accent, no glow.
  const overheat = pct > 70;

  return (
    <View
      style={styles.row}
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(clampedPct) }}
      accessibilityLabel={`context used ${formatTokenPair(used, total)}`}
    >
      <View style={[styles.track, { backgroundColor: tokens.border }]}>
        <Animated.View
          style={[
            styles.fill,
            { backgroundColor: fillColor },
            overheat &&
              shadowStyle({
                shadowColor: fillColor,
                shadowOpacity: 0.6,
                shadowRadius: 4,
                shadowOffset: { width: 0, height: 0 },
                elevation: 0,
              }),
            fillStyle,
          ]}
        />
      </View>
      <Text style={[typeScale.meta, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>
        {formatTokenPair(used, total)}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
  },
  track: {
    flex: 1,
    height: TRACK_HEIGHT,
    borderRadius: radii.radiusPill,
    // Not `overflow: "hidden"` — the fill carries its own matching radius, and an
    // overheat glow (Move 1) needs to bleed a few px beyond the 3px track.
  },
  fill: {
    height: "100%",
    borderRadius: radii.radiusPill,
  },
});
