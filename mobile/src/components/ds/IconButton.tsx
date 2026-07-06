// DESIGN_SYSTEM.md §6 IconButton — 44x44 hit area, 20px icon, D/P/F/X, optional badge dot.
import React, { useState } from "react";
import { Pressable, StyleSheet, View, type StyleProp, type ViewStyle } from "react-native";
import Animated from "react-native-reanimated";

import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, tapTarget } from "../../theme/tokens";

export interface IconButtonProps {
  /** Caller renders the lucide icon at size 20 / stroke 1.75, colored to match context. */
  icon: React.ReactNode;
  onPress?: () => void;
  disabled?: boolean;
  badge?: boolean;
  accessibilityLabel: string;
  testID?: string;
  style?: StyleProp<ViewStyle>;
}

export function IconButton({
  icon,
  onPress,
  disabled = false,
  badge = false,
  accessibilityLabel,
  testID,
  style,
}: IconButtonProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [focused, setFocused] = useState(false);

  return (
    <Animated.View style={strike.style}>
      <Pressable
        onPress={disabled ? undefined : onPress}
        onPressIn={disabled ? undefined : strike.onPressIn}
        onPressOut={disabled ? undefined : strike.onPressOut}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        disabled={disabled}
        testID={testID}
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel}
        accessibilityState={{ disabled }}
        style={[
          styles.base,
          {
            borderRadius: radii.radius8,
            opacity: disabled ? 0.4 : 1,
            borderColor: focused ? tokens.accent : "transparent",
          },
          style,
        ]}
      >
        <View style={styles.iconWrap}>{icon}</View>
        {badge ? (
          <View style={[styles.badge, { backgroundColor: tokens.danger, borderColor: tokens.bg1 }]} />
        ) : null}
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  base: {
    width: tapTarget,
    height: tapTarget,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 2,
  },
  iconWrap: {
    alignItems: "center",
    justifyContent: "center",
  },
  badge: {
    position: "absolute",
    top: 6,
    right: 6,
    width: 8,
    height: 8,
    borderRadius: 4,
    borderWidth: 1,
  },
});
