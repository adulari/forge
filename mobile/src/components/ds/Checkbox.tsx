// DESIGN_SYSTEM.md §6 Checkbox(worktree toggle) — accent when on.
import React, { useState } from "react";
import { Pressable, StyleSheet, View, type StyleProp, type ViewStyle } from "react-native";
import { Check } from "lucide-react-native";
import Animated from "react-native-reanimated";

import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";

export interface CheckboxProps {
  value: boolean;
  onValueChange: (value: boolean) => void;
  disabled?: boolean;
  accessibilityLabel: string;
  testID?: string;
  style?: StyleProp<ViewStyle>;
}

const SIZE = 22;

export function Checkbox({ value, onValueChange, disabled = false, accessibilityLabel, testID, style }: CheckboxProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [focused, setFocused] = useState(false);

  return (
    <Animated.View style={strike.style}>
      <Pressable
        onPress={disabled ? undefined : () => onValueChange(!value)}
        onPressIn={disabled ? undefined : strike.onPressIn}
        onPressOut={disabled ? undefined : strike.onPressOut}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        disabled={disabled}
        testID={testID}
        accessibilityRole="checkbox"
        accessibilityState={{ checked: value, disabled }}
        accessibilityLabel={accessibilityLabel}
        hitSlop={11}
        style={[styles.hit, style]}
      >
        <View
          style={[
            styles.box,
            {
              width: SIZE,
              height: SIZE,
              borderRadius: radii.radius4,
              backgroundColor: value ? tokens.accent : "transparent",
              borderColor: focused ? tokens.accent : value ? tokens.accent : tokens.borderStrong,
              opacity: disabled ? 0.4 : 1,
            },
          ]}
        >
          {value ? <Check size={16} strokeWidth={2} color={tokens.onAccent} /> : null}
        </View>
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  hit: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  box: {
    borderWidth: 2,
    alignItems: "center",
    justifyContent: "center",
  },
});
