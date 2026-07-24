// Machined Segmented — bg2 track with 1px border (radii 4 outer / 2 inner), and a STATIC
// selected segment: a neutral low-alpha fill (not accent-tinted — Machined keeps the
// single ember accent state-only) with an accent label, exactly as every prototype
// segmented renders (Usage THIS WEEK, Settings LIGHT/DARK/SYSTEM, Forge-a-task
// READ/ASK/EDIT/FULL). No sliding thumb: the motion set has no segmented slide, and the
// old measured-thumb animation was the source of repeated misalignment.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTheme } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

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
  /** Kept for source compatibility: flush tracks sit on a bg2 header surface, so they drop
   * the track fill and keep only the border. */
  flush?: boolean;
}

export function Segmented<T extends string = string>({ options, value, onChange, testID, flush }: SegmentedProps<T>) {
  const { tokens } = useTheme();

  return (
    <View
      style={[
        styles.track,
        {
          backgroundColor: flush ? "transparent" : tokens.bg2,
          borderColor: tokens.border,
          borderRadius: radii.radiusSegmentOuter,
        },
      ]}
      testID={testID}
      accessibilityRole="tablist"
    >
      {options.map((opt) => (
        <SegmentOption
          key={opt.value}
          label={opt.label}
          badge={opt.badge}
          dot={opt.dot}
          selected={opt.value === value}
          onPress={() => onChange(opt.value)}
        />
      ))}
    </View>
  );
}

// Own component (not inline in the `.map`) so each segment can carry its own
// hover/focus-visible state — hooks can't run conditionally/per-iteration
// inside a parent's render body.
function SegmentOption({
  label,
  badge,
  dot,
  selected,
  onPress,
}: {
  label: string;
  badge?: number;
  dot?: boolean;
  selected: boolean;
  onPress: () => void;
}) {
  const { scheme, tokens } = useTheme();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);

  // Machined active-segment fill: a neutral low-alpha overlay, not an accent tint —
  // the ember accent stays state-only (busy/waiting/etc), never a persistent selection
  // color. Dark's value equals `tokens.border` exactly; light has no existing token at
  // this exact alpha, so both are spelled out explicitly here (design spec literal).
  const activeFill = scheme === "light" ? "rgba(0,0,0,0.08)" : "rgba(244,244,246,0.09)";

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
          borderWidth: 1,
          borderColor: focused ? tokens.accent : "transparent",
          backgroundColor: selected ? activeFill : hovered ? tokens.bg3 : "transparent",
        },
      ]}
    >
      <View style={styles.labelRow}>
        <Text
          style={[
            type.section,
            styles.monoLabel,
            { color: selected ? tokens.accent : tokens.ink3 },
          ]}
          numberOfLines={1}
        >
          {label}
        </Text>
        {badge != null && badge > 0 ? (
          <View style={[styles.badge, { backgroundColor: selected ? activeFill : tokens.bg3 }]}>
            <Text style={[type.meta, styles.monoBadge, { color: selected ? tokens.accent : tokens.ink2 }]}>
              {badge}
            </Text>
          </View>
        ) : null}
        {dot ? <View style={[styles.dot, { backgroundColor: tokens.accent }]} /> : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  track: {
    flexDirection: "row",
    padding: 3,
    minHeight: 44,
    borderWidth: 1,
  },
  labelRow: { flexDirection: "row", alignItems: "center", gap: 4 },
  // 11px semibold mono for short technical option labels (READ/ASK/EDIT/FULL,
  // LIGHT/DARK/SYSTEM, ...) — overrides `type.section`'s 10px sans family/size,
  // keeps its letterSpacing/uppercase.
  monoLabel: { fontSize: 11, lineHeight: 14, fontFamily: monoFamily.bold },
  monoBadge: { fontFamily: monoFamily.regular },
  badge: { minWidth: 16, height: 16, paddingHorizontal: 4, borderRadius: 8, alignItems: "center", justifyContent: "center" },
  dot: { width: 6, height: 6, borderRadius: 3 },
  segment: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
  },
});
