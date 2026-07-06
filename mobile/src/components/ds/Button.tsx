// DESIGN_SYSTEM.md §6 Button — variants primary/secondary/ghost/danger/allow.
// States: D default · P pressed (Strike) · F focused (2px accent ring) ·
// L loading (spinner-in-place, label persists at 0.6) · X disabled (0.4, no Strike).
import React, { useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import Animated from "react-native-reanimated";

import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "allow";

export interface ButtonProps {
  label: string;
  onPress?: () => void;
  variant?: ButtonVariant;
  loading?: boolean;
  disabled?: boolean;
  fullWidth?: boolean;
  icon?: React.ReactNode;
  testID?: string;
  accessibilityLabel?: string;
  style?: StyleProp<ViewStyle>;
}

export function Button({
  label,
  onPress,
  variant = "primary",
  loading = false,
  disabled = false,
  fullWidth = false,
  icon,
  testID,
  accessibilityLabel,
  style,
}: ButtonProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [focused, setFocused] = useState(false);
  const [hovered, setHovered] = useState(false);
  const isDisabled = disabled || loading;

  let bg: string;
  let ink: string;
  switch (variant) {
    case "primary":
      bg = tokens.accent;
      ink = tokens.onAccent;
      break;
    case "secondary":
      bg = tokens.bg3;
      ink = tokens.ink;
      break;
    case "ghost":
      bg = hovered ? tokens.bg3 : "transparent";
      ink = tokens.ink2;
      break;
    case "danger":
      bg = tokens.danger;
      ink = tokens.onAccent;
      break;
    case "allow":
      bg = tokens.success;
      ink = tokens.onAccent;
      break;
  }

  const minHeight = variant === "primary" ? 48 : tapTarget;

  return (
    <Animated.View style={[strike.style, fullWidth && styles.fullWidth]}>
      <Pressable
        onPress={isDisabled ? undefined : onPress}
        onPressIn={isDisabled ? undefined : strike.onPressIn}
        onPressOut={isDisabled ? undefined : strike.onPressOut}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        disabled={isDisabled}
        testID={testID}
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel ?? label}
        accessibilityState={{ disabled: isDisabled, busy: loading }}
        style={[
          styles.base,
          {
            backgroundColor: bg,
            minHeight,
            borderRadius: radii.radius8,
            opacity: isDisabled ? 0.4 : 1,
            borderColor: focused ? tokens.accent : "transparent",
          },
          style,
        ]}
      >
        <View style={[styles.content, { opacity: loading ? 0.6 : 1 }]}>
          {loading ? (
            <ActivityIndicator size="small" color={ink} style={styles.spinner} />
          ) : icon ? (
            <View style={styles.icon}>{icon}</View>
          ) : null}
          <Text style={[type.bodyBold, { color: ink }]} numberOfLines={1}>
            {label}
          </Text>
        </View>
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  base: {
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: space.space16,
    borderWidth: 2,
  },
  content: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
  },
  icon: {
    marginRight: space.space4,
  },
  spinner: {
    marginRight: space.space8,
  },
  fullWidth: {
    width: "100%",
  },
});
