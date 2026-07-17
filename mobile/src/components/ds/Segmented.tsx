// DESIGN_SYSTEM.md §6 Segmented — bg3 track, bg2 thumb (Tabshift: thumb slides
// with `press` spring), section-style labels; used for Chat/Tasks/Agents/Review.
// DESIGN_ELEVATION.md Move 3 — 1px inset hairline on the thumb ("machined" edge).
// Hearth radii: 10 outer / 7 inner (`radiusSegmentOuter`/`radiusSegmentInner`).
import React, { useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withSpring } from "react-native-reanimated";

import { springs } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface SegmentedOption<T extends string = string> {
  value: T;
  label: string;
  badge?: number;
  dot?: boolean;
}

export interface SegmentedProps<T extends string = string> {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  testID?: string;
  /** When true, the track uses bg2 (matching the surrounding header surface) so the
   * segmented strip blends flush into a uniform header — the selected thumb pops as a
   * raised chip using bg3. Use this when Segmented is rendered inside a header that
   * already paints bg2 as its background. */
  flush?: boolean;
}

export function Segmented<T extends string = string>({ options, value, onChange, testID, flush }: SegmentedProps<T>) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const [width, setWidth] = useState(0);
  const translateX = useSharedValue(0);
  // Tracks whether the thumb has been placed once at its measured width yet —
  // that first placement snaps instantly (no spring "slide-in" from the left
  // edge on mount); every switch after that runs the `press` spring.
  const hasPlaced = useRef(false);
  const index = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const segmentWidth = options.length > 0 ? (width - 4) / options.length : 0;

  // flush: track = bg2 (header surface), thumb = bg3 (raised chip).
  // default:  track = bg3,          thumb = bg2 (existing section-style look).
  const trackBg = flush ? tokens.bg2 : tokens.bg3;
  const thumbBg = flush ? tokens.bg3 : tokens.bg2;

  useEffect(() => {
    if (segmentWidth <= 0) return;
    const target = index * segmentWidth;
    if (reduced || !hasPlaced.current) {
      translateX.value = target;
      hasPlaced.current = true;
    } else {
      translateX.value = withSpring(target, springs.press);
    }
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
      style={[styles.track, { backgroundColor: trackBg, borderRadius: radii.radiusSegmentOuter }]}
      testID={testID}
      accessibilityRole="tablist"
    >
      {width > 0 ? (
        <Animated.View
          pointerEvents="none"
          style={[styles.thumb, thumbStyle, { backgroundColor: thumbBg, borderRadius: radii.radiusSegmentInner }]}
        >
          <View
            style={[
              styles.thumbInset,
              { borderColor: tokens.borderStrong, borderRadius: radii.radiusSegmentInner - 1, pointerEvents: "none" },
            ]}
          />
        </Animated.View>
      ) : null}
      {options.map((opt) => {
        const selected = opt.value === value;
        return (
          <SegmentOption
            key={opt.value}
            label={opt.label}
            badge={opt.badge}
            dot={opt.dot}
            selected={selected}
            onPress={() => onChange(opt.value)}
          />
        );
      })}
    </View>
  );
}

// Own component (not inline in the `.map`) so each segment can carry its own
// hover/focus-visible state — hooks can't run conditionally/per-iteration
// inside a parent's render body.
function SegmentOption({ label, badge, dot, selected, onPress }: { label: string; badge?: number; dot?: boolean; selected: boolean; onPress: () => void }) {
  const tokens = useTokens();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);

  return (
    <Pressable
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      accessibilityRole="tab"
      accessibilityState={{ selected }}
      accessibilityLabel={label}
      style={[
        styles.segment,
        {
          borderRadius: radii.radiusSegmentInner,
          borderWidth: 2,
          borderColor: focused ? tokens.accent : "transparent",
          backgroundColor: hovered && !selected ? tokens.bg3 : "transparent",
        },
      ]}
    >
      <View style={styles.labelRow}>
        <Text style={[type.section, { color: selected ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>{label}</Text>
        {badge != null && badge > 0 ? <View style={[styles.badge, { backgroundColor: selected ? tokens.selection : tokens.bg3 }]}><Text style={[type.meta, { color: selected ? tokens.accent : tokens.ink2 }]}>{badge}</Text></View> : null}
        {dot ? <View style={[styles.dot, { backgroundColor: tokens.accent }]} /> : null}
      </View>
    </Pressable>
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
    left: 2,
  },
  thumbInset: {
    position: "absolute",
    top: 1,
    left: 1,
    right: 1,
    bottom: 1,
    borderWidth: StyleSheet.hairlineWidth,
  },
  labelRow: { flexDirection: "row", alignItems: "center", gap: 4 },
  badge: { minWidth: 16, height: 16, paddingHorizontal: 4, borderRadius: 8, alignItems: "center", justifyContent: "center" },
  dot: { width: 6, height: 6, borderRadius: 3 },
  segment: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1,
  },
});
