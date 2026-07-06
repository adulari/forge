// Motion primitives shared by every animated surface. Every animation here respects
// reduce-motion (UI_RULES.md #17): when enabled, entrances render in their final state
// and pulses render solid — never a spring, never a loop.
import { useEffect, useRef, useState } from "react";
import {
  cancelAnimation,
  Easing,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withDelay,
  withRepeat,
  withSequence,
  withTiming,
} from "react-native-reanimated";

import { motionDurationMs, pulse } from "./theme";

/** Re-export so callers don't need to import reanimated directly for this one check. */
export function useMotionEnabled(): boolean {
  return !useReducedMotion();
}

const ENTRANCE_STAGGER_MS = 40;
const ENTRANCE_TRANSLATE_Y = 8;

/**
 * Staggered entrance: fade + slight translateY, offset by `index * stagger`.
 * Renders instantly (opacity 1, no transform) when motion is disabled.
 */
export function useEntranceAnimation(index: number, stagger = ENTRANCE_STAGGER_MS) {
  const motionEnabled = useMotionEnabled();
  const opacity = useSharedValue(motionEnabled ? 0 : 1);
  const translateY = useSharedValue(motionEnabled ? ENTRANCE_TRANSLATE_Y : 0);

  useEffect(() => {
    if (!motionEnabled) {
      opacity.value = 1;
      translateY.value = 0;
      return;
    }
    const delay = Math.min(index, 12) * stagger;
    opacity.value = withDelay(
      delay,
      withTiming(1, { duration: motionDurationMs, easing: Easing.out(Easing.quad) }),
    );
    translateY.value = withDelay(
      delay,
      withTiming(0, { duration: motionDurationMs, easing: Easing.out(Easing.quad) }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [motionEnabled, index, stagger]);

  return useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }],
  }));
}

const PRESS_SCALE = 0.97;

/** Press-scale feedback: scales to 0.97 on pressIn, back to 1 on pressOut/cancel. */
export function usePressScale() {
  const motionEnabled = useMotionEnabled();
  const scale = useSharedValue(1);

  const style = useAnimatedStyle(() => ({
    transform: [{ scale: scale.value }],
  }));

  const onPressIn = () => {
    scale.value = motionEnabled
      ? withTiming(PRESS_SCALE, { duration: 80 })
      : PRESS_SCALE;
  };
  const onPressOut = () => {
    scale.value = motionEnabled ? withTiming(1, { duration: 120 }) : 1;
  };

  return { style, onPressIn, onPressOut };
}

/** Count-up for numeric Metrics (cost, tokens). Snaps instantly when motion is disabled. */
export function useCountUp(value: number, duration = motionDurationMs): number {
  const motionEnabled = useMotionEnabled();
  const [display, setDisplay] = useState(value);
  const fromRef = useRef(value);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    if (!motionEnabled) {
      setDisplay(value);
      fromRef.current = value;
      return;
    }
    const from = fromRef.current;
    const to = value;
    if (from === to) return;
    const start = Date.now();
    const tick = () => {
      const elapsed = Date.now() - start;
      const t = Math.min(1, elapsed / duration);
      const eased = 1 - (1 - t) * (1 - t);
      setDisplay(from + (to - from) * eased);
      if (t < 1) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        fromRef.current = to;
      }
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, motionEnabled, duration]);

  return display;
}

export type PulseKind = "busy" | "waiting";

/**
 * Status-dot pulse. `busy` = 1s cycle, `waiting` = 0.7s (faster), opacity 1 -> 0.35 -> 1.
 * Renders solid (opacity 1, no loop) when motion is disabled (UI_RULES.md #17).
 */
export function usePulse(kind: PulseKind) {
  const motionEnabled = useMotionEnabled();
  const opacity = useSharedValue(1);
  const durationMs =
    kind === "busy" ? pulse.busyDurationMs : pulse.waitingDurationMs;

  useEffect(() => {
    if (!motionEnabled) {
      cancelAnimation(opacity);
      opacity.value = 1;
      return;
    }
    opacity.value = withRepeat(
      withSequence(
        withTiming(pulse.minOpacity, {
          duration: durationMs / 2,
          easing: Easing.inOut(Easing.ease),
        }),
        withTiming(1, {
          duration: durationMs / 2,
          easing: Easing.inOut(Easing.ease),
        }),
      ),
      -1,
    );
    return () => cancelAnimation(opacity);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [motionEnabled, durationMs]);

  return useAnimatedStyle(() => ({ opacity: opacity.value }));
}
