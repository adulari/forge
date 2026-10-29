// The session header's socket-state chip: Live, Reconnecting, Paused in background, Offline.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { SocketChipModel, SocketChipTone } from "../../lib/socketChip";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type ColorTokens } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

function toneColor(tone: SocketChipTone, tokens: ColorTokens): string {
  switch (tone) {
    case "ok":
      return tokens.success;
    case "warn":
      return tokens.warn;
    case "danger":
      return tokens.danger;
    default:
      return tokens.ink3;
  }
}

export function SocketChip({ model, compact }: { model: SocketChipModel; compact: boolean }) {
  const tokens = useTokens();
  const color = toneColor(model.tone, tokens);
  // A phone's header has no room for a word that only repeats the normal state.
  const dotOnly = compact && model.tone === "ok";
  return (
    <View
      style={[styles.chip, { borderColor: tokens.border }, dotOnly && styles.dotOnly]}
      accessible
      accessibilityLabel={`Connection: ${model.label}`}
      accessibilityLiveRegion="polite"
      testID="socket-chip"
    >
      <View style={[styles.dot, { backgroundColor: color }]} />
      {dotOnly ? null : (
        <Text style={[typeScale.meta, { color: tokens.ink2 }]} numberOfLines={1}>
          {model.label}
        </Text>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space8,
    paddingVertical: 2,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radiusPill,
    flexShrink: 0,
  },
  dotOnly: { paddingHorizontal: space.space4, borderColor: "transparent" },
  dot: { width: 6, height: 6, borderRadius: 3 },
});
