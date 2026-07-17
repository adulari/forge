// DESIGN_SYSTEM.md §6 Button — variants primary/secondary/ghost/danger/allow.
// States: D default · P pressed (Strike) · F focused (2px accent ring) ·
// L loading (spinner-in-place, label persists at 0.6) · X disabled (0.4, no Strike).
// Hearth core rule 4 (fixed semantic map, never swapped): "allow" is a filled
// success button with `successBg` ink (not the generic `onAccent`); "danger" is the
// outlined "deny" look — transparent fill, `borderStrong` border, `danger` ink.
// Radius 10-12 (Hearth radii scale) for every variant.
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
  accessibilityHint?: string;
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
  accessibilityHint,
  style,
}: ButtonProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [focused, setFocused] = useState(false);
  const [hovered, setHovered] = useState(false);
  const isDisabled = disabled || loading;

  let bg: string;
  let ink: string;
  let border = "transparent";
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
      // Hearth "Deny": outlined, never a filled red button.
      bg = "transparent";
      ink = tokens.danger;
      border = tokens.borderStrong;
      break;
    case "allow":
      // Hearth "Allow": success fill with successBg ink, not the generic onAccent.
      bg = tokens.success;
      ink = tokens.successBg;
      break;
  }

  // DESIGN_ELEVATION.md Move 3 — disabled: flat `bg3` fill + `ink4` label for
  // every variant, not a dimmed accent (which read muddy-brown). This is the
  // `disabled` prop specifically — `loading` keeps its own variant color and
  // dims only the inner content (spinner/label) below, per §6's L/X states.
  if (disabled) {
    bg = tokens.bg3;
    ink = tokens.ink4;
    border = "transparent";
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
        accessibilityHint={accessibilityHint}
        accessibilityState={{ disabled: isDisabled, busy: loading }}
        style={[
          styles.base,
          {
            backgroundColor: bg,
            minHeight,
            borderRadius: radii.radius12,
            borderColor: focused ? tokens.accent : border,
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
