// DESIGN_SYSTEM.md §6 Status & data: `KeyValueRow` — settings rows: label ink,
// value ink2, optional chevron. Pressable rows use Strike (§5.2: "every Pressable
// in ds/") via `useStrike`.
import { ChevronRight } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useStrike } from "../../theme/motion";
import { rowHeight, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface KeyValueRowProps {
  label: string;
  value?: string;
  onPress?: () => void;
  chevron?: boolean;
  accessibilityLabel?: string;
}

export function KeyValueRow({ label, value, onPress, chevron = false, accessibilityLabel }: KeyValueRowProps) {
  const tokens = useTokens();
  const { style: strikeStyle, onPressIn, onPressOut } = useStrike();

  const row = (
    <Animated.View
      style={[styles.row, { borderBottomColor: tokens.border }, onPress ? strikeStyle : null]}
    >
      <Text style={[typeScale.body, styles.label, { color: tokens.ink }]} numberOfLines={1}>
        {label}
      </Text>
      <View style={styles.trailing}>
        {value != null ? (
          <Text style={[typeScale.body, styles.value, { color: tokens.ink2 }]} numberOfLines={1}>
            {value}
          </Text>
        ) : null}
        {chevron ? <ChevronRight size={20} strokeWidth={1.75} color={tokens.ink3} /> : null}
      </View>
    </Animated.View>
  );

  if (!onPress) return row;

  return (
    <Pressable
      onPress={onPress}
      onPressIn={onPressIn}
      onPressOut={onPressOut}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? label}
    >
      {row}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    minHeight: rowHeight.dense,
    paddingHorizontal: space.space16,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: space.space12,
  },
  // A long value (e.g. a full server hostname) otherwise refuses to shrink — RN's Text
  // defaults to flexShrink: 0, so `space-between` collapses to zero gap and the label and
  // value render touching, with numberOfLines={1} never getting a chance to ellipsize.
  label: { flexShrink: 0 },
  trailing: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    flexShrink: 1,
  },
  value: { flexShrink: 1 },
});
