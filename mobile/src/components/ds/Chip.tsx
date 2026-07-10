// DESIGN_SYSTEM.md §6 Chip — pill radius 999, bg3, meta text; selectable state
// (selection bg + ember/accent text); used for command chips + filters.
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
}

export function Chip({ label, selected = false, onPress, disabled = false, icon, testID, style }: ChipProps) {
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
        hitSlop={{ top: 6, bottom: 6, left: 4, right: 4 }}
        style={[
          styles.base,
          {
            backgroundColor: selected ? tokens.selection : tokens.bg3,
            borderRadius: radii.radiusPill,
            opacity: disabled ? 0.4 : 1,
            borderWidth: 2,
            borderColor: focused ? tokens.accent : hovered && !disabled ? tokens.borderStrong : "transparent",
          },
          style,
        ]}
      >
        {icon ? <View style={styles.icon}>{icon}</View> : null}
        <Text style={[type.meta, { color: selected ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
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
  },
  icon: {
    marginRight: space.space4,
  },
});
