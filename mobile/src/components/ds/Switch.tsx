// DESIGN_SYSTEM.md §6 Switch — accent when on.
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, type StyleProp, type ViewStyle } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";

import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";

export interface SwitchProps {
  value: boolean;
  onValueChange: (value: boolean) => void;
  disabled?: boolean;
  accessibilityLabel: string;
  testID?: string;
  style?: StyleProp<ViewStyle>;
}

const TRACK_W = 44;
const TRACK_H = 26;
const THUMB = 22;
const PAD = 2;

export function Switch({ value, onValueChange, disabled = false, accessibilityLabel, testID, style }: SwitchProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const progress = useSharedValue(value ? 1 : 0);
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    progress.value = reduced
      ? value
        ? 1
        : 0
      : withTiming(value ? 1 : 0, { duration: durations.fast, easing: easings.standard });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, reduced]);

  const thumbStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: progress.value * (TRACK_W - THUMB - PAD * 2) }],
  }));
  const trackStyle = useAnimatedStyle(() => ({
    backgroundColor: progress.value > 0.5 ? tokens.accent : tokens.bg3,
  }));

  return (
    <Pressable
      onPress={disabled ? undefined : () => onValueChange(!value)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      disabled={disabled}
      testID={testID}
      accessibilityRole="switch"
      accessibilityState={{ checked: value, disabled }}
      accessibilityLabel={accessibilityLabel}
      hitSlop={8}
      style={[styles.hit, { opacity: disabled ? 0.4 : 1 }, style]}
    >
      <Animated.View
        style={[styles.track, trackStyle, { borderWidth: focused ? 2 : 0, borderColor: tokens.accent }]}
      >
        <Animated.View
          style={[styles.thumb, thumbStyle, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        />
      </Animated.View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  hit: {
    width: TRACK_W,
    height: TRACK_H,
  },
  track: {
    width: TRACK_W,
    height: TRACK_H,
    borderRadius: TRACK_H / 2,
    padding: PAD,
    justifyContent: "center",
  },
  thumb: {
    width: THUMB,
    height: THUMB,
    borderRadius: THUMB / 2,
    borderWidth: 1,
  },
});
