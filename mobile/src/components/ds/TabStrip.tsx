// Hearth flat tab strip (Session Chat/Tasks/Agents/Review) — type-first text tabs on a
// hairline baseline: active = ink + 2px ember underline, inactive = ink3, optional mono
// count suffix or waiting dot. Distinct from Segmented (the pill control stays for true
// value pickers like temper/appearance); these are navigation tabs.
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, tabularNums } from "../../theme/typography";

export interface TabStripOption<T extends string> {
  value: T;
  label: string;
  /** Mono count rendered after the label (omitted when undefined). */
  badge?: number;
  /** Waiting/attention dot rendered after the label. */
  dot?: boolean;
}

export interface TabStripProps<T extends string> {
  options: TabStripOption<T>[];
  value: T;
  onChange: (value: T) => void;
  testID?: string;
}

export function TabStrip<T extends string>({ options, value, onChange, testID }: TabStripProps<T>) {
  const tokens = useTokens();
  return (
    <View style={[styles.row, { borderBottomColor: tokens.hairline }]} testID={testID} accessibilityRole="tablist">
      {options.map((option) => {
        const active = option.value === value;
        return (
          <Pressable
            key={option.value}
            onPress={() => onChange(option.value)}
            accessibilityRole="tab"
            accessibilityState={{ selected: active }}
            accessibilityLabel={
              option.badge != null ? `${option.label}, ${option.badge}` : option.label
            }
            style={styles.tab}
            hitSlop={{ top: 8, bottom: 8 }}
          >
            <View style={styles.labelRow}>
              <Text style={[styles.label, { color: active ? tokens.ink : tokens.ink3 }]}>{option.label}</Text>
              {option.badge != null ? (
                <Text style={[styles.badge, tabularNums, { color: tokens.ink4 }]}>{option.badge}</Text>
              ) : null}
              {option.dot ? <View style={[styles.dot, { backgroundColor: tokens.danger }]} /> : null}
            </View>
            {active ? <View style={[styles.underline, { backgroundColor: tokens.accent }]} /> : null}
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    gap: space.space20,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  tab: { position: "relative", paddingBottom: 10 },
  labelRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  label: { fontSize: 12.5, fontWeight: "600", letterSpacing: 0.4 },
  badge: { fontFamily: monoFamily.regular, fontSize: 11 },
  dot: { width: 5, height: 5, borderRadius: 3 },
  underline: { position: "absolute", left: 0, right: 0, bottom: 0, height: 2 },
});
