// DESIGN_SYSTEM.md §6 Input — bg2, border, radius 8, 15pt; label (meta, ink3) above;
// D/F/E/X; mono variant for URLs/paths; `clear` affordance.
import React, { useState } from "react";
import {
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextInputProps,
  type ViewStyle,
} from "react-native";
import { X } from "lucide-react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { monoFamily, type, webInputTextStyle } from "../../theme/typography";

export interface InputProps extends Omit<TextInputProps, "style"> {
  label?: string;
  error?: string;
  disabled?: boolean;
  /** Mono variant for URLs/paths/tokens. */
  mono?: boolean;
  /** Shows a clear (X) affordance when there is text. Default true. */
  clearable?: boolean;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
  containerStyle?: StyleProp<ViewStyle>;
  testID?: string;
}

export function Input({
  label,
  error,
  disabled = false,
  mono = false,
  clearable = true,
  leading,
  trailing,
  value,
  onChangeText,
  containerStyle,
  testID,
  accessibilityLabel,
  onFocus,
  onBlur,
  ...rest
}: InputProps) {
  const tokens = useTokens();
  const [focused, setFocused] = useState(false);
  const [clearFocused, setClearFocused] = useState(false);
  const hasError = !!error;
  const showClear = clearable && !trailing && !disabled && !!value && value.length > 0;

  // DESIGN_SYSTEM §6 state legend: F = "2px accent ring, web/desktop; native: borderStrong".
  const webFocusRing = focused && !hasError && Platform.OS === "web";
  const borderColor = hasError ? tokens.danger : webFocusRing ? tokens.accent : focused ? tokens.borderStrong : tokens.border;

  return (
    <View style={containerStyle}>
      {label ? <Text style={[type.meta, { color: tokens.ink3, marginBottom: space.space4 }]}>{label}</Text> : null}
      <View
        style={[
          styles.field,
          {
            backgroundColor: tokens.bg2,
            borderColor,
            borderWidth: webFocusRing ? 2 : 1,
            borderRadius: radii.radius8,
            opacity: disabled ? 0.4 : 1,
          },
        ]}
      >
        {leading ? <View style={styles.slot}>{leading}</View> : null}
        <TextInput
          {...rest}
          value={value}
          onChangeText={onChangeText}
          editable={!disabled}
          onFocus={(e) => {
            setFocused(true);
            onFocus?.(e);
          }}
          onBlur={(e) => {
            setFocused(false);
            onBlur?.(e);
          }}
          placeholderTextColor={tokens.ink3}
          style={[
            styles.input,
            type.body,
            webInputTextStyle,
            { color: tokens.ink, fontFamily: mono ? monoFamily.regular : undefined },
          ]}
          testID={testID}
          accessibilityLabel={accessibilityLabel ?? label}
        />
        {showClear ? (
          <Pressable
            onPress={() => onChangeText?.("")}
            onFocus={() => setClearFocused(true)}
            onBlur={() => setClearFocused(false)}
            accessibilityRole="button"
            accessibilityLabel="Clear"
            style={[styles.clear, { borderColor: clearFocused ? tokens.accent : "transparent" }]}
          >
            <X size={16} strokeWidth={1.75} color={tokens.ink3} />
          </Pressable>
        ) : trailing ? (
          <View style={styles.slot}>{trailing}</View>
        ) : null}
      </View>
      {hasError ? <Text style={[type.sub, { color: tokens.danger, marginTop: space.space4 }]}>{error}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  field: {
    flexDirection: "row",
    alignItems: "center",
    borderWidth: 1,
    minHeight: 44,
    paddingHorizontal: space.space12,
  },
  slot: {
    marginHorizontal: space.space4,
  },
  clear: {
    width: tapTarget,
    height: tapTarget,
    marginRight: -space.space8,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 2,
    borderRadius: radii.radius8,
  },
  input: {
    flex: 1,
    paddingVertical: space.space8,
  },
});
