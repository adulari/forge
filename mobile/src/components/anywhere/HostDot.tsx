// Forge Anywhere host status dot — design comp "host status dot + text" legend
// (mobile.dc.html lines 1302-1313). Reuses the existing Emberdot pulse (ds/StatusDot's
// pattern, theme/motion.ts useEmberdot) rather than inventing a new animation: busy/
// connecting map onto Emberdot's "busy" pulse, everything else renders static.
import React from "react";
import { StyleSheet, View } from "react-native";
import Animated from "react-native-reanimated";

import type { HostState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { useEmberdot } from "../../theme/motion";
import type { ColorTokens } from "../../theme/tokens";

const DOT_SIZE = 7;

type DotColorKey = keyof Pick<ColorTokens, "success" | "accent" | "warn" | "ink4" | "danger">;

function dotColorKey(state: HostState): DotColorKey {
  switch (state.kind) {
    case "online":
      return state.activity === "busy" ? "accent" : "success";
    case "connecting":
      return "accent";
    case "stale":
    case "update-required":
      return "warn";
    case "offline":
    case "disabled":
      return "ink4";
    case "revoked":
      return "danger";
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

function isPulsing(state: HostState): boolean {
  return state.kind === "connecting" || (state.kind === "online" && state.activity === "busy");
}

export interface HostDotProps {
  state: HostState;
  size?: number;
  accessibilityLabel?: string;
}

export function HostDot({ state, size = DOT_SIZE, accessibilityLabel }: HostDotProps) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(isPulsing(state) ? "busy" : "idle");
  const color = tokens[dotColorKey(state)];

  return (
    <View
      style={[styles.wrap, { width: size, height: size }]}
      accessibilityRole="image"
      accessibilityLabel={accessibilityLabel ?? `host: ${state.kind}`}
    >
      <Animated.View
        style={[styles.dot, { width: size, height: size, borderRadius: size / 2, backgroundColor: color }, dotStyle]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { alignItems: "center", justifyContent: "center" },
  dot: {},
});
