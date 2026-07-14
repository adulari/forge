// DESIGN_SYSTEM.md §6 Status & data: `ContextGauge` — 3px track (border color) +
// fill animated via Gaugeflow (§5.2), color steps accent -> warn (>70%) -> danger
// (>90%) per §1.4, `128.4k/200k` meta beside via `formatTokenPair`.
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useGaugeflow, useThermal } from "../../theme/motion";
import { gaugeColor, radii, shadowStyle, space } from "../../theme/tokens";
import { formatTokenPair, tabularNums, type as typeScale } from "../../theme/typography";

export interface ContextGaugeProps {
  used: number;
  total: number;
  compact?: boolean;
}

const TRACK_HEIGHT = 4;

export function ContextGauge({ used, total, compact = false }: ContextGaugeProps) {
  const tokens = useTokens();
  const rawPct = total > 0 && Number.isFinite(used) ? (used / total) * 100 : 0;
  const pct = Math.max(0, Math.min(100, rawPct));
  const { style: fillStyle } = useGaugeflow(pct);
  const fillColor = gaugeColor(pct, tokens);
  const overheat = pct > 70;
  const thermalStyle = useThermal(pct > 90 ? "busy" : "off");

  return (
    <View
      style={[styles.row, compact && styles.compactRow]}
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(pct) }}
      accessibilityLabel={`context used ${formatTokenPair(used, total)}`}
    >
      <View style={[styles.track, { backgroundColor: tokens.border }]}>
        <Animated.View style={thermalStyle}>
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
        </Animated.View>
      </View>
      <Text style={[typeScale.meta, tabularNums, { color: pct > 70 ? fillColor : tokens.ink3 }]} numberOfLines={1}>
        {compact ? `${Math.round(pct)}%` : formatTokenPair(used, total)}
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
  compactRow: { flex: 0, minWidth: 78, gap: space.space4 },
  track: {
    flex: 1,
    minWidth: 0,
    height: TRACK_HEIGHT,
    overflow: "hidden",
    borderRadius: radii.radiusPill,
    // Not `overflow: "hidden"` — the fill carries its own matching radius, and an
    // overheat glow (Move 1) needs to bleed a few px beyond the 3px track.
  },
  fill: {
    height: "100%",
    borderRadius: radii.radiusPill,
  },
});
