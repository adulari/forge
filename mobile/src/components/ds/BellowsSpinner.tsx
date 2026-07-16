// DESIGN_SYSTEM.md §5.2 "Bellows" (pull-to-refresh): "ember arc that fills 0->270deg with
// pull distance, then rotates while refreshing; settles with a single soft haptic."
//
// RN's native RefreshControl doesn't expose the raw pull-gesture distance (iOS/Android own
// that overscroll internally), so the 0->270deg "fill with pull" phase isn't something a JS
// overlay can drive faithfully without replacing the native pull gesture outright — too risky
// to bolt onto every list in the app (Fleet/Inbox/History all share BoundedList) without a
// device to verify against. What IS deliverable and verified here: the moment refreshing
// starts, this ember arc fades in over the (now-hidden, iOS `tintColor="transparent"`) native
// spinner and rotates continuously — the "then rotates while refreshing" half of the spec,
// rendered in the same instrument-arc material as TaskRow's in_progress glyph and
// ContextGauge's fill. BoundedList wires the settle haptic on the refreshing:true->false edge.
import React, { useEffect } from "react";
import { StyleSheet, View } from "react-native";
import Svg, { Circle } from "react-native-svg";
import Animated, {
  cancelAnimation,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { easings } from "../../theme/motion";

const SIZE = 22;
const STROKE = 2.5;
const RADIUS = (SIZE - STROKE) / 2;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;
// A 270deg arc, matching the spec's full-pull fill angle.
const ARC_FRACTION = 270 / 360;
const SPIN_MS = 900;

export interface BellowsSpinnerProps {
  /** True while the list is refreshing. Renders nothing otherwise. */
  active: boolean;
}

export function BellowsSpinner({ active }: BellowsSpinnerProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const rotation = useSharedValue(0);

  useEffect(() => {
    cancelAnimation(rotation);
    if (!active || reduced) {
      rotation.value = 0;
      return;
    }
    rotation.value = withRepeat(withTiming(360, { duration: SPIN_MS, easing: easings.linear }), -1, false);
    return () => cancelAnimation(rotation);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, reduced]);

  // Rotate the SVG's centered box instead of setting react-native-svg's `origin` prop on a shape.
  // On web, react-native-svg 15.15 converts that prop to the invalid React DOM property
  // `transform-origin`, which emits a warning under React 19. A View transform has the same visual
  // centre on every platform and stays entirely within React Native's supported style surface.
  const spinStyle = useAnimatedStyle(() => ({
    transform: [{ rotate: `${rotation.value}deg` }],
  }));

  if (!active) return null;

  return (
    <View
      style={[styles.wrap, { pointerEvents: "none" }]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      <Animated.View style={[styles.spinner, spinStyle]}>
        <Svg width={SIZE} height={SIZE} viewBox={`0 0 ${SIZE} ${SIZE}`}>
          <Circle
            cx={SIZE / 2}
            cy={SIZE / 2}
            r={RADIUS}
            stroke={tokens.accent}
            strokeWidth={STROKE}
            strokeLinecap="round"
            strokeDasharray={`${CIRCUMFERENCE * ARC_FRACTION} ${CIRCUMFERENCE}`}
            fill="none"
          />
        </Svg>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  spinner: {
    width: SIZE,
    height: SIZE,
  },
  wrap: {
    position: "absolute",
    top: 10,
    left: 0,
    right: 0,
    alignItems: "center",
    zIndex: 1,
  },
});
