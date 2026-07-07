// DESIGN_SYSTEM.md §5.2 "Approve/Deny commit": "...a small check/x icon draws in (SVG
// stroke, 200ms)". Used as the `icon` slot of the tapped Allow/Deny/Approve/Cancel button on
// PermissionCard/PlanCard once a choice is locked in — scale+fade mount animation standing in
// for a true stroke-draw (Button's `icon` prop is a plain node, not an SVG path this can
// progressively reveal), gated by reduce-motion like every other Forgework pattern.
import { Check, X } from "lucide-react-native";
import React, { useEffect } from "react";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";

import { durations, easings } from "../../theme/motion";

const ICON_SIZE = 16;
const ICON_STROKE = 2;

export interface CommitIconProps {
  kind: "check" | "x";
  color: string;
}

export function CommitIcon({ kind, color }: CommitIconProps) {
  const reduced = useReducedMotion();
  const progress = useSharedValue(reduced ? 1 : 0);

  useEffect(() => {
    progress.value = reduced ? 1 : withTiming(1, { duration: durations.base, easing: easings.standard });
    // Mount-only: a CommitIcon instance is only ever rendered once a decision locks in.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const style = useAnimatedStyle(() => ({
    opacity: progress.value,
    transform: [{ scale: 0.5 + progress.value * 0.5 }],
  }));

  const Icon = kind === "check" ? Check : X;

  return (
    <Animated.View style={style}>
      <Icon size={ICON_SIZE} strokeWidth={ICON_STROKE} color={color} />
    </Animated.View>
  );
}
