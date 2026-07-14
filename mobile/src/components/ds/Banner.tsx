// DESIGN_SYSTEM.md §6 Containers / §5.2 Signal — Banner: tones warn/danger/
// neutral, slides down from the header `base`. The "reconnecting" neutral
// strip variant (`compact`) is 12pt meta, no animation.
import React, { useEffect } from "react";
import { StyleSheet, Text, type ViewStyle } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { durations, easings } from "../../theme/motion";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export type BannerTone = "warn" | "danger" | "neutral";

export interface BannerProps {
  tone: BannerTone;
  message: string;
  /** The "reconnecting" strip: 12pt meta text, no animation. Default false. */
  compact?: boolean;
  visible?: boolean;
  style?: ViewStyle;
}

const SLIDE_PX = 8;

export function Banner({ tone, message, compact = false, visible = true, style }: BannerProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const translateY = useSharedValue(compact || reduced ? 0 : visible ? 0 : -SLIDE_PX);
  const opacity = useSharedValue(compact || reduced ? (visible ? 1 : 0) : visible ? 1 : 0);

  useEffect(() => {
    if (compact) return; // the reconnecting strip never animates
    if (reduced) {
      opacity.value = visible ? 1 : 0;
      translateY.value = 0;
      return;
    }
    opacity.value = withTiming(visible ? 1 : 0, { duration: durations.base, easing: easings.standard });
    translateY.value = withTiming(visible ? 0 : -SLIDE_PX, { duration: durations.base, easing: easings.standard });
  }, [visible, reduced, compact, opacity, translateY]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }],
  }));

  if (!visible && compact) return null;

  const backgroundColor = tone === "danger" ? tokens.dangerBg : tone === "warn" ? tokens.warnBg : tokens.bg3;
  const ink = tone === "danger" ? tokens.danger : tone === "warn" ? tokens.warnBgInk : tokens.ink2;

  return (
    <Animated.View
      style={[styles.base, { backgroundColor }, compact ? styles.compact : styles.regular, animatedStyle, style]}
      accessibilityRole="alert"
      accessibilityLabel={message}
    >
      <Text style={[compact ? type.meta : type.sub, styles.message, { color: ink }]}>{message}</Text>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  base: { width: "100%" },
  regular: { paddingHorizontal: space.space16, paddingVertical: space.space12 },
  compact: { paddingHorizontal: space.space16, paddingVertical: space.space4 },
  message: { flexShrink: 1 },
});
