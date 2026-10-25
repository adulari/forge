// DESIGN_SYSTEM.md §6 Chip — pill radius 999, meta text; selectable state
// (selection bg + ember/accent text); used for command chips + filters. Hearth:
// unselected chips are bg2 with a visible 1px border (the command-chip strip in
// Session Chat), not a borderless bg3 fill.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import Animated from "react-native-reanimated";

import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface ChipProps {
  label: string;
  selected?: boolean;
  onPress?: () => void;
  disabled?: boolean;
  icon?: React.ReactNode;
  testID?: string;
  style?: StyleProp<ViewStyle>;
  /** `compact`: a 30px pill with a hairline border for dense rows (the composer's command
   * chips); the hit area keeps its tap target through hitSlop. */
  size?: "regular" | "compact";
}

export function Chip({ label, selected = false, onPress, disabled = false, icon, testID, style, size = "regular" }: ChipProps) {
  const compact = size === "compact";
  const tokens = useTokens();
  const strike = useStrike();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);

  return (
    <Animated.View style={strike.style}>
      <Pressable
        onPress={disabled ? undefined : onPress}
        onPressIn={disabled ? undefined : strike.onPressIn}
        onPressOut={disabled ? undefined : strike.onPressOut}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        disabled={disabled}
        testID={testID}
        accessibilityRole="button"
        accessibilityState={{ disabled, selected }}
        accessibilityLabel={label}
        hitSlop={compact ? { top: 8, bottom: 8, left: 2, right: 2 } : { top: 6, bottom: 6, left: 4, right: 4 }}
        style={[
          styles.base,
          compact ? styles.compact : null,
          {
            backgroundColor: selected ? tokens.selection : tokens.bg2,
            borderRadius: radii.radiusPill,
            opacity: disabled ? 0.4 : 1,
            borderWidth: compact ? StyleSheet.hairlineWidth * 2 : 2,
            borderColor: focused
              ? tokens.accent
              : selected
                ? "transparent"
                : hovered && !disabled
                  ? tokens.borderStrong
                  : tokens.border,
          },
          style,
        ]}
      >
        {icon ? <View style={styles.icon}>{icon}</View> : null}
          <Text style={[type.meta, styles.label, { color: selected ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
          {label}
        </Text>
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  base: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: space.space12,
    minHeight: tapTarget,
    maxWidth: "100%",
    flexShrink: 1,
  },
  compact: { minHeight: 30, paddingHorizontal: space.space8 + 2 },
  label: { flexShrink: 1 },
  icon: {
    marginRight: space.space4,
  },
});
