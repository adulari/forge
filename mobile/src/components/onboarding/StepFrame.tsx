// The card every step renders into: what it is for, what was found, the step's own controls,
// and the way out (back, skip). The step's single primary button lives in `children`.
import React, { useEffect } from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";

import { detectedLines, showsNew } from "../../lib/onboarding/flow";
import type { PlanStep, Snapshot } from "../../lib/onboarding/types";
import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { FactList } from "./parts";

export interface StepFrameProps {
  step: PlanStep;
  snapshot: Snapshot;
  position: { index: number; total: number };
  children: React.ReactNode;
  onBack?: () => void;
  onSkip?: () => void;
  skipLabel?: string;
  /** Lines to lead with instead of the standard ones. */
  detected?: string[];
}

export function StepFrame({ step, snapshot, position, children, onBack, onSkip, skipLabel = "Skip this step", detected }: StepFrameProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const { isCompact: compact } = useBreakpoint();
  // Each step is its own component, so this runs once per step: a short fade in, none under
  // reduced motion. (Reanimated's `entering` presets pull the layout-animation runtime into the
  // web bundle, which is far heavier than this.)
  const opacity = useSharedValue(reduced ? 1 : 0);
  useEffect(() => {
    if (!reduced) opacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
  }, [opacity, reduced]);
  const fade = useAnimatedStyle(() => ({ opacity: opacity.value }));
  const lines = detected ?? detectedLines(step.id, snapshot);
  const blocking = step.required && snapshot.missing.includes(step.id);
  return (
    <Animated.View
      style={[styles.card, compact && styles.cardCompact, { backgroundColor: tokens.bg1, borderColor: tokens.border }, fade]}
      testID={`onboarding-step-${step.id}`}
    >
      <View style={styles.head}>
        <View style={styles.eyebrowRow}>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>
            {String(position.index + 1).padStart(2, "0")} / {String(position.total).padStart(2, "0")}
          </Text>
          {step.required ? <Badge label={blocking ? "Required" : "Required · done"} tone={blocking ? "warn" : "success"} /> : null}
          {showsNew(step, snapshot) ? <Badge label="New" tone="accent" /> : null}
        </View>
        <Text style={[type.display, { color: tokens.ink }]} accessibilityRole="header">
          {step.title}
        </Text>
        <Text style={[type.body, { color: tokens.ink2 }]}>{step.summary}</Text>
      </View>
      <FactList lines={lines} />
      <View style={styles.body}>{children}</View>
      {onBack || onSkip ? (
        <View style={[styles.foot, { borderTopColor: tokens.hairline }]}>
          {onBack ? <Button label="Back" variant="ghost" onPress={onBack} /> : <View />}
          {onSkip ? <Button label={skipLabel} variant="ghost" onPress={onSkip} /> : null}
        </View>
      ) : null}
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space24, gap: space.space20 },
  cardCompact: { padding: space.space16, gap: space.space16 },
  head: { gap: space.space8 },
  eyebrowRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  body: { gap: space.space16 },
  foot: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space12 },
});
