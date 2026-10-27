// Machined Stepper: ordered progress through a flow. Three shapes of the same data:
//  - "dots": numbered dots in a row with labels (the Forge Anywhere sign-in header),
//  - "rail": a vertical list for a desktop side column, rows selectable,
//  - "bar":  one line of text over a segmented bar (phone-width top progress).
import { Check, Minus } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { stepperStatuses, type StepperStatus, type StepperStep } from "../../lib/stepper";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export type { StepperStatus, StepperStep };

export interface StepperProps {
  steps: StepperStep[];
  activeIndex?: number;
  variant?: "dots" | "rail" | "bar";
  onSelect?: (key: string, index: number) => void;
  testID?: string;
}

export function Stepper({ steps, activeIndex = 0, variant = "dots", onSelect, testID }: StepperProps) {
  const statuses = stepperStatuses(steps, activeIndex);
  const current = Math.max(0, statuses.indexOf("current"));
  const accessibilityValue = { min: 1, max: Math.max(steps.length, 1), now: current + 1 };
  if (variant === "rail") {
    return (
      <View testID={testID} accessibilityRole="list" style={styles.rail}>
        {steps.map((step, index) => (
          <RailRow
            key={step.key}
            step={step}
            status={statuses[index]}
            index={index}
            last={index === steps.length - 1}
            onSelect={onSelect}
          />
        ))}
      </View>
    );
  }
  if (variant === "bar") {
    return <StepBar steps={steps} statuses={statuses} current={current} testID={testID} />;
  }
  return (
    <View
      testID={testID}
      style={styles.dots}
      accessibilityRole="progressbar"
      accessibilityValue={accessibilityValue}
    >
      {steps.map((step, index) => (
        <View key={step.key} style={styles.dotsItem}>
          <Dot status={statuses[index]} index={index} />
          <DotLabel label={step.label} status={statuses[index]} />
        </View>
      ))}
    </View>
  );
}

function DotLabel({ label, status }: { label: string; status: StepperStatus }) {
  const tokens = useTokens();
  return (
    <Text style={[type.meta, { color: status === "current" ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>
      {label}
    </Text>
  );
}

function Dot({ status, index }: { status: StepperStatus; index: number }) {
  const tokens = useTokens();
  const done = status === "done";
  const current = status === "current";
  return (
    <View
      style={[
        styles.dot,
        {
          backgroundColor: done ? tokens.success : current ? tokens.accent : tokens.bg3,
          borderColor: done ? tokens.success : current ? tokens.accent : tokens.borderStrong,
        },
      ]}
    >
      {done ? (
        <Check size={12} strokeWidth={2.5} color={tokens.successBg} />
      ) : status === "skipped" ? (
        <Minus size={12} strokeWidth={2.5} color={tokens.ink3} />
      ) : (
        <Text style={[type.monoMeta, { color: current ? tokens.onAccent : tokens.ink3 }]}>{index + 1}</Text>
      )}
    </View>
  );
}

function RailRow({
  step,
  status,
  index,
  last,
  onSelect,
}: {
  step: StepperStep;
  status: StepperStatus;
  index: number;
  last: boolean;
  onSelect?: (key: string, index: number) => void;
}) {
  const tokens = useTokens();
  const [hovered, setHovered] = React.useState(false);
  const current = status === "current";
  const ink = current ? tokens.ink : status === "done" ? tokens.ink2 : tokens.ink3;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${step.label}${step.required ? ", required" : ""}, ${status === "current" ? "current step" : status}`}
      accessibilityState={{ selected: current }}
      disabled={!onSelect}
      onPress={() => onSelect?.(step.key, index)}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      style={[
        styles.railRow,
        {
          backgroundColor: current ? tokens.selection : hovered && onSelect ? tokens.bg2 : "transparent",
          borderRadius: radii.radius8,
        },
      ]}
    >
      <View style={styles.railDotColumn}>
        <Dot status={status} index={index} />
        {last ? null : <View style={[styles.railLine, { backgroundColor: status === "done" ? hexToRgba(tokens.success, 0.5) : tokens.border }]} />}
      </View>
      <View style={styles.railText}>
        <Text style={[type.bodyBold, { color: ink }]} numberOfLines={1}>
          {step.label}
        </Text>
        {step.required && status !== "done" ? (
          <Text style={[type.meta, { color: tokens.warn }]}>Required</Text>
        ) : step.isNew ? (
          <Text style={[type.meta, { color: tokens.accent }]}>New</Text>
        ) : null}
      </View>
    </Pressable>
  );
}

function StepBar({
  steps,
  statuses,
  current,
  testID,
}: {
  steps: StepperStep[];
  statuses: StepperStatus[];
  current: number;
  testID?: string;
}) {
  const tokens = useTokens();
  return (
    <View
      testID={testID}
      accessibilityRole="progressbar"
      accessibilityValue={{ min: 1, max: Math.max(steps.length, 1), now: current + 1 }}
      style={styles.bar}
    >
      <Text style={[type.meta, { color: tokens.ink3 }]}>
        Step {current + 1} of {steps.length}
        {steps[current] ? `  ·  ${steps[current].label}` : ""}
      </Text>
      <View style={styles.barSegments}>
        {statuses.map((status, index) => (
          <View
            key={steps[index].key}
            style={[
              styles.barSegment,
              {
                backgroundColor:
                  status === "done" ? tokens.success : status === "current" ? tokens.accent : tokens.bg3,
              },
            ]}
          />
        ))}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  dots: { flexDirection: "row", marginTop: space.space24, marginBottom: space.space20, justifyContent: "space-between" },
  dotsItem: { flex: 1, alignItems: "center", gap: space.space4 },
  dot: { width: 26, height: 26, borderRadius: 13, borderWidth: 1, alignItems: "center", justifyContent: "center" },
  rail: { gap: 0 },
  railRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space12, paddingHorizontal: space.space8, paddingVertical: space.space4, minHeight: 44 },
  railDotColumn: { alignItems: "center", width: 26, paddingTop: 2 },
  railLine: { width: 1, flex: 1, minHeight: 10, marginTop: 2 },
  railText: { flex: 1, paddingTop: 3, paddingBottom: space.space8, gap: 0 },
  bar: { gap: space.space8 },
  barSegments: { flexDirection: "row", gap: 3 },
  barSegment: { flex: 1, height: 3, borderRadius: 2 },
});
