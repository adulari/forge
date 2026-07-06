// DESIGN_SYSTEM.md §6 Segmented — bg3 track, bg2 thumb (Tabshift: thumb slides
// with `press` spring), section-style labels; used for Chat/Tasks/Agents/Review.
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withSpring } from "react-native-reanimated";

import { springs } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface SegmentedOption<T extends string = string> {
  value: T;
  label: string;
}

export interface SegmentedProps<T extends string = string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  testID?: string;
}

export function Segmented<T extends string = string>({ options, value, onChange, testID }: SegmentedProps<T>) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const [width, setWidth] = useState(0);
  const translateX = useSharedValue(0);
  const index = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const segmentWidth = options.length > 0 ? width / options.length : 0;

  useEffect(() => {
    const target = index * segmentWidth;
    translateX.value = reduced ? target : withSpring(target, springs.press);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [index, segmentWidth, reduced]);

  const thumbStyle = useAnimatedStyle(() => ({
    transform: [{ translateX: translateX.value }],
    width: segmentWidth,
  }));

  const onLayout = (e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width);

  return (
    <View
      onLayout={onLayout}
      style={[styles.track, { backgroundColor: tokens.bg3, borderRadius: radii.radius8 }]}
      testID={testID}
      accessibilityRole="tablist"
    >
      {width > 0 ? (
        <Animated.View
          style={[styles.thumb, thumbStyle, { backgroundColor: tokens.bg2, borderRadius: radii.radius8 - 2 }]}
        />
      ) : null}
      {options.map((opt) => {
        const selected = opt.value === value;
        return (
          <Pressable
            key={opt.value}
            onPress={() => onChange(opt.value)}
            accessibilityRole="tab"
            accessibilityState={{ selected }}
            accessibilityLabel={opt.label}
            style={styles.segment}
          >
            <Text style={[type.section, { color: selected ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>
              {opt.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  track: {
    flexDirection: "row",
    position: "relative",
    padding: 2,
    minHeight: 44,
  },
  thumb: {
    position: "absolute",
    top: 2,
    bottom: 2,
    left: 0,
  },
  segment: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1,
  },
});
