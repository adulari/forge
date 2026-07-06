// DESIGN_SYSTEM.md §6 Containers / §5.2 Temper — Skeleton: shimmer blocks that
// match the final layout (fleet rows, history rows, chat bubbles, ...).
import React, { useState } from "react";
import { StyleSheet, View, type DimensionValue, type ViewStyle } from "react-native";
import Animated, { interpolate, useAnimatedStyle } from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useTemper } from "../../theme/motion";
import { radii } from "../../theme/tokens";

export interface SkeletonProps {
  width?: DimensionValue;
  height?: number;
  radius?: number;
  style?: ViewStyle;
}

const BAND_WIDTH = 80;

/** A single Temper shimmer block. Compose several to build a screen-shaped skeleton. */
export function Skeleton({ width = "100%", height = 16, radius = radii.radius4, style }: SkeletonProps) {
  const tokens = useTokens();
  const { progress, active } = useTemper();
  const [measuredWidth, setMeasuredWidth] = useState(0);

  const shimmerStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: interpolate(progress.value, [0, 1], [-BAND_WIDTH, measuredWidth || BAND_WIDTH]) }],
  }));

  return (
    <View
      style={[styles.base, { width, height, borderRadius: radius, backgroundColor: tokens.bg3 }, style]}
      onLayout={(e) => setMeasuredWidth(e.nativeEvent.layout.width)}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {active ? (
        <Animated.View
          style={[styles.shimmer, { backgroundColor: tokens.border, width: BAND_WIDTH }, shimmerStyle]}
        />
      ) : null}
    </View>
  );
}

/** A common composite: avatar + two lines, shaped like a fleet/history row. */
export function SkeletonRow({ style }: { style?: ViewStyle }) {
  return (
    <View style={[styles.row, style]}>
      <Skeleton width={40} height={40} radius={radii.radiusPill} />
      <View style={styles.rowBody}>
        <Skeleton width="60%" height={14} />
        <Skeleton width="40%" height={12} style={styles.rowGap} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  base: { overflow: "hidden" },
  shimmer: { position: "absolute", top: 0, bottom: 0, opacity: 0.35 },
  row: { flexDirection: "row", alignItems: "center", gap: 12, paddingVertical: 8 },
  rowBody: { flex: 1 },
  rowGap: { marginTop: 6 },
});
