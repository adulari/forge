// DESIGN_SYSTEM.md §6 SearchField — Input + search icon + cancel, debounced 150ms.
import React, { useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { Search } from "lucide-react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Input, type InputProps } from "./Input";

export interface SearchFieldProps
  extends Omit<InputProps, "leading" | "trailing" | "clearable" | "value" | "onChangeText"> {
  value: string;
  onChangeText: (text: string) => void;
  /** Fired `debounceMs` after the last keystroke — for server-side filtering. */
  onDebouncedChange?: (text: string) => void;
  debounceMs?: number;
  onCancel?: () => void;
  showCancel?: boolean;
}

export function SearchField({
  value,
  onChangeText,
  onDebouncedChange,
  debounceMs = 150,
  onCancel,
  showCancel = true,
  placeholder = "Search",
  containerStyle,
  accessibilityLabel,
  ...rest
}: SearchFieldProps) {
  const tokens = useTokens();
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [cancelFocused, setCancelFocused] = useState(false);

  useEffect(() => {
    if (!onDebouncedChange) return;
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => onDebouncedChange(value), debounceMs);
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [value, debounceMs, onDebouncedChange]);

  const cancelVisible = showCancel && value.length > 0;

  return (
    <View style={[styles.row, containerStyle]}>
      <Input
        value={value}
        onChangeText={onChangeText}
        placeholder={placeholder}
        leading={<Search size={16} strokeWidth={1.75} color={tokens.ink3} />}
        containerStyle={styles.field}
        accessibilityLabel={accessibilityLabel ?? placeholder}
        {...rest}
      />
      {cancelVisible ? (
        <Pressable
          onPress={() => {
            onChangeText("");
            onCancel?.();
          }}
          hitSlop={8}
          onFocus={() => setCancelFocused(true)}
          onBlur={() => setCancelFocused(false)}
          accessibilityRole="button"
          accessibilityLabel="Cancel search"
          style={[styles.cancel, { borderColor: cancelFocused ? tokens.accent : "transparent" }]}
        >
          <Text style={[type.body, { color: tokens.accent }]}>Cancel</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "center",
  },
  field: {
    flex: 1,
  },
  cancel: {
    marginLeft: space.space12,
    minHeight: 44,
    borderWidth: 2,
    borderRadius: 8,
  },
});
