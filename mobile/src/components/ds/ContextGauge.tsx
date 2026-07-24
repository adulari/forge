// DESIGN_SYSTEM.md §6 Status & data: `ContextGauge` — 2px stroke track (a fixed
// neutral overlay, not a theme border token — see `TRACK_COLOR` below) + fill
// animated via Gaugeflow (§5.2), color steps accent -> warn (>70%) -> danger (>90%)
// per §1.4, `128.4k/200k` mono beside via `formatTokenPair`. Machined drops the old
// overheat glow shadow (thermal identity retired) — the color step alone signals it.
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTheme } from "../../theme/ThemeProvider";
import { useGaugeflow } from "../../theme/motion";
import { gaugeColor, radii, space } from "../../theme/tokens";
import { formatTokenPair, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

export interface ContextGaugeProps {
  used: number;
  total: number;
  compact?: boolean;
  /** Hearth core rule 7 ("context as % in glanceable chrome"): append " ctx" after the
   * compact percentage (`22% ctx`). Default false — existing bare "NN%" callers unaffected. */
  ctxLabel?: boolean;
}

const TRACK_HEIGHT = 2;

export function ContextGauge({ used, total, compact = false, ctxLabel = false }: ContextGaugeProps) {
  const { scheme, tokens } = useTheme();
  const rawPct = total > 0 && Number.isFinite(used) ? (used / total) * 100 : 0;
  const pct = Math.max(0, Math.min(100, rawPct));
  const { style: fillStyle } = useGaugeflow(pct);
  const fillColor = gaugeColor(pct, tokens);
  // Fixed neutral track overlay (design spec literal — dark equals `tokens.border`
  // exactly; light has no existing token at this alpha, see Segmented's same pattern).
  const trackColor = scheme === "light" ? "rgba(0,0,0,0.08)" : "rgba(244,244,246,0.08)";

  return (
    <View
      style={[styles.row, compact && styles.compactRow]}
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 0, max: 100, now: Math.round(pct) }}
      accessibilityLabel={`context used ${formatTokenPair(used, total)}`}
    >
      <View style={[styles.track, { backgroundColor: trackColor }]}>
        <Animated.View style={[styles.fill, { backgroundColor: fillColor }, fillStyle]} />
      </View>
      <Text
        style={[typeScale.meta, styles.mono, tabularNums, { color: pct > 70 ? fillColor : tokens.ink3 }]}
        numberOfLines={1}
      >
        {compact ? `${Math.round(pct)}%${ctxLabel ? " ctx" : ""}` : formatTokenPair(used, total)}
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
  mono: { fontFamily: monoFamily.regular },
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
