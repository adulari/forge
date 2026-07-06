// DESIGN_SYSTEM.md §5 ("Forgework"): motion tokens + the named pattern hooks.
// Every hook here checks useReducedMotion() and renders the final state statically
// when it's on — pulses go solid, entrances render instantly, springs snap with no
// animation. Never gate this check behind anything else.
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
  withSpring,
  withTiming,
} from "react-native-reanimated";

// ---------------------------------------------------------------------------
// §5.1 Tokens
// ---------------------------------------------------------------------------

export const durations = {
  instant: 80,
  fast: 140,
  base: 200,
  gentle: 260,
  sheet: 320,
} as const;

export type DurationToken = keyof typeof durations;

/** Raw cubic-bezier control points, for the web CSS twin (`transition-timing-function`). */
export const easingCurves = {
  standard: [0.2, 0, 0, 1],
  exit: [0.3, 0, 1, 1],
  linear: [0, 0, 1, 1],
} as const;

/** Reanimated Easing functions for native `withTiming`. */
export const easings = {
  standard: Easing.bezier(...easingCurves.standard),
  exit: Easing.bezier(...easingCurves.exit),
  linear: Easing.linear,
};

export const springs = {
  press: { damping: 30, stiffness: 500 },
  sheet: { damping: 28, stiffness: 260, mass: 0.9 },
  emphasis: { damping: 16, stiffness: 200 },
} as const;

/** Re-export so callers don't need to import reanimated directly for this one check. */
export function useMotionEnabled(): boolean {
  return !useReducedMotion();
}

// ---------------------------------------------------------------------------
// Strike (press) — every Pressable in ds/.
// scale->0.97 + opacity->0.9, `press` spring in, 120ms timing out.
// ---------------------------------------------------------------------------

const STRIKE_SCALE = 0.97;
const STRIKE_OPACITY = 0.9;
const STRIKE_OUT_MS = 120;

export function useStrike() {
  const reduced = useReducedMotion();
  const scale = useSharedValue(1);
  const opacity = useSharedValue(1);

  const style = useAnimatedStyle(() => ({
    transform: [{ scale: scale.value }],
    opacity: opacity.value,
  }));

  const onPressIn = () => {
    if (reduced) {
      scale.value = STRIKE_SCALE;
      opacity.value = STRIKE_OPACITY;
      return;
    }
    scale.value = withSpring(STRIKE_SCALE, springs.press);
    opacity.value = withSpring(STRIKE_OPACITY, springs.press);
  };

  const onPressOut = () => {
    if (reduced) {
      scale.value = 1;
      opacity.value = 1;
      return;
    }
    scale.value = withTiming(1, { duration: STRIKE_OUT_MS, easing: easings.standard });
    opacity.value = withTiming(1, { duration: STRIKE_OUT_MS, easing: easings.standard });
  };

  return { style, onPressIn, onPressOut };
}

// ---------------------------------------------------------------------------
// Forgeline (list entrance) — rows fade + translateY 8->0, base/standard,
// stagger 40ms, capped at 8 rows, first mount of a screen only.
// ---------------------------------------------------------------------------

const FORGELINE_STAGGER_MS = 40;
const FORGELINE_TRANSLATE_Y = 8;
const FORGELINE_CAP = 8;

export function useForgeline(index: number) {
  const reduced = useReducedMotion();
  const opacity = useSharedValue(reduced ? 1 : 0);
  const translateY = useSharedValue(reduced ? 0 : FORGELINE_TRANSLATE_Y);

  useEffect(() => {
    if (reduced) {
      opacity.value = 1;
      translateY.value = 0;
      return;
    }
    const delay = Math.min(index, FORGELINE_CAP) * FORGELINE_STAGGER_MS;
    opacity.value = withDelay(delay, withTiming(1, { duration: durations.base, easing: easings.standard }));
    translateY.value = withDelay(delay, withTiming(0, { duration: durations.base, easing: easings.standard }));
    // First-mount-only is a caller contract (stable keys, no remount on refresh) —
    // this hook intentionally re-runs the entrance if `index` changes identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reduced, index]);

  return useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }],
  }));
}

// ---------------------------------------------------------------------------
// Emberdot (status) — busy: opacity 1<->0.35 @1s; waiting: @0.7s + a 1.5px
// danger ring that scales 1->1.6 and fades, every 2.8s. Idle/done: static.
// ---------------------------------------------------------------------------

export type EmberdotKind = "busy" | "waiting" | "idle" | "done";

const EMBERDOT_MIN_OPACITY = 0.35;
const EMBERDOT_BUSY_MS = 1000;
const EMBERDOT_WAITING_MS = 700;
const EMBERDOT_RING_PERIOD_MS = 2800;
const EMBERDOT_RING_ANIM_MS = 1200;

export function useEmberdot(kind: EmberdotKind) {
  const reduced = useReducedMotion();
  const opacity = useSharedValue(1);
  const ringScale = useSharedValue(1);
  const ringOpacity = useSharedValue(0);

  useEffect(() => {
    cancelAnimation(opacity);
    cancelAnimation(ringScale);
    cancelAnimation(ringOpacity);

    if (reduced || (kind !== "busy" && kind !== "waiting")) {
      opacity.value = 1;
      ringScale.value = 1;
      ringOpacity.value = 0;
      return;
    }

    const pulseMs = kind === "busy" ? EMBERDOT_BUSY_MS : EMBERDOT_WAITING_MS;
    opacity.value = withRepeat(
      withSequence(
        withTiming(EMBERDOT_MIN_OPACITY, { duration: pulseMs / 2, easing: easings.standard }),
        withTiming(1, { duration: pulseMs / 2, easing: easings.standard }),
      ),
      -1,
    );

    if (kind === "waiting") {
      ringScale.value = 1;
      ringOpacity.value = 0.8;
      ringScale.value = withRepeat(
        withSequence(
          withTiming(1.6, { duration: EMBERDOT_RING_ANIM_MS, easing: easings.standard }),
          withTiming(1, { duration: EMBERDOT_RING_PERIOD_MS - EMBERDOT_RING_ANIM_MS }),
        ),
        -1,
      );
      ringOpacity.value = withRepeat(
        withSequence(
          withTiming(0, { duration: EMBERDOT_RING_ANIM_MS, easing: easings.standard }),
          withTiming(0.8, { duration: EMBERDOT_RING_PERIOD_MS - EMBERDOT_RING_ANIM_MS }),
        ),
        -1,
      );
    }

    return () => {
      cancelAnimation(opacity);
      cancelAnimation(ringScale);
      cancelAnimation(ringOpacity);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, reduced]);

  const dotStyle = useAnimatedStyle(() => ({ opacity: opacity.value }));
  const ringStyle = useAnimatedStyle(() => ({
    opacity: ringOpacity.value,
    transform: [{ scale: ringScale.value }],
  }));

  return { dotStyle, ringStyle };
}

// ---------------------------------------------------------------------------
// Gaugeflow (context/cost) — gauge width animates over `gentle`.
// Color is a threshold step (accent/warn/danger via tokens.gaugeColor), not
// interpolated here — callers re-render with the new color, no smooth blend.
// ---------------------------------------------------------------------------

export function useGaugeflow(pct: number) {
  const reduced = useReducedMotion();
  const clamped = Math.max(0, Math.min(100, pct));
  const value = useSharedValue(clamped);

  useEffect(() => {
    value.value = reduced
      ? clamped
      : withTiming(clamped, { duration: durations.gentle, easing: easings.standard });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clamped, reduced]);

  const style = useAnimatedStyle(() => ({ width: `${value.value}%` }));
  return { style };
}

// ---------------------------------------------------------------------------
// Temper (skeleton -> content) — 1.6s linear shimmer sweep while loading.
// The Skeleton component (T1.3) knows its own measured width, so this hook
// hands back the raw 0->1 sweep progress rather than guessing pixel geometry;
// the component maps it to `translateX: interpolate(progress.value, [0, 1],
// [-bandWidth, containerWidth])` across its shimmer band.
// ---------------------------------------------------------------------------

const TEMPER_SHIMMER_MS = 1600;

export function useTemper() {
  const reduced = useReducedMotion();
  const progress = useSharedValue(0);

  useEffect(() => {
    cancelAnimation(progress);
    if (reduced) {
      progress.value = 0;
      return;
    }
    progress.value = withRepeat(withTiming(1, { duration: TEMPER_SHIMMER_MS, easing: easings.linear }), -1, false);
    return () => cancelAnimation(progress);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reduced]);

  return { progress, active: !reduced };
}

// ---------------------------------------------------------------------------
// Count-up (cost/token metrics) — ported unchanged from lib/motion.ts.
// ---------------------------------------------------------------------------

export function useCountUp(value: number, duration: number = durations.base): number {
  const reduced = useReducedMotion();
  const [display, setDisplay] = useState(value);
  const fromRef = useRef(value);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    if (reduced) {
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
     
  }, [value, reduced, duration]);

  return display;
}
