// DESIGN_SYSTEM.md §6 Status & data: `StatusDot(state)` — 8px, Emberdot behavior
// (§5.2/§1.4): busy pulses @1s, waiting pulses @0.7s + a danger ring beacon every
// 2.8s, idle/done are static. Color mapping comes straight from `statusDotColor`.
import React from "react";
import { StyleSheet, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useEmberdot } from "../../theme/motion";
import { statusDotColor, type StatusDotState } from "../../theme/tokens";

const DOT_SIZE = 8;
const RING_BORDER_WIDTH = 1.5;

export interface StatusDotProps {
  state: StatusDotState;
  accessibilityLabel?: string;
}

export function StatusDot({ state, accessibilityLabel }: StatusDotProps) {
  const tokens = useTokens();
  const { dotStyle, ringStyle } = useEmberdot(state);
  const color = statusDotColor(state, tokens);

  return (
    <View
      style={styles.wrap}
      accessibilityRole="image"
      accessibilityLabel={accessibilityLabel ?? `status: ${state}`}
    >
      {state === "waiting" ? (
        <Animated.View
          pointerEvents="none"
          style={[styles.ring, { borderColor: tokens.danger }, ringStyle]}
        />
      ) : null}
      <Animated.View style={[styles.dot, { backgroundColor: color }, dotStyle]} />
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
  ring: {
    position: "absolute",
    width: DOT_SIZE,
    height: DOT_SIZE,
    borderRadius: DOT_SIZE / 2,
    borderWidth: RING_BORDER_WIDTH,
  },
});
