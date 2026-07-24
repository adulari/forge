// DESIGN_SYSTEM.md §6 Status & data: `StatusDot(state)` — 6px, flat (no glow/halo).
// Machined retires the busy glow halo and waiting ring beacon; busy/waiting still
// pulse in place via `useEmberdot`'s own opacity animation (the dot's state motion,
// not a decorative glow). Color mapping comes straight from `statusDotColor`.
import React from "react";
import { StyleSheet, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useEmberdot } from "../../theme/motion";
import { statusDotColor, type StatusDotState } from "../../theme/tokens";

const DOT_SIZE = 6;

export interface StatusDotProps {
  state: StatusDotState;
  size?: number;
  accessibilityLabel?: string;
}

export function StatusDot({ state, size = DOT_SIZE, accessibilityLabel }: StatusDotProps) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(state);
  const color = statusDotColor(state, tokens);

  return (
    <View
      style={[styles.wrap, { width: size, height: size }]}
      accessibilityRole="image"
      accessibilityLabel={accessibilityLabel ?? `status: ${state}`}
    >
      <Animated.View style={[styles.dot, { width: size, height: size, borderRadius: size / 2, backgroundColor: color }, dotStyle]} />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    width: DOT_SIZE,
    height: DOT_SIZE,
    alignItems: "center",
    justifyContent: "center",
  },
  dot: {
    width: DOT_SIZE,
    height: DOT_SIZE,
    borderRadius: DOT_SIZE / 2,
  },
});
