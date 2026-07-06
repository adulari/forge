// DESIGN_SYSTEM.md §6 Containers — Screen: safe-area, bg1, gutter, optional
// scroll + keyboard-avoid. ONE per route.
import React from "react";
import {
  KeyboardAvoidingView,
  Platform,
  ScrollView,
  type ScrollViewProps,
  StyleSheet,
  View,
  type ViewStyle,
} from "react-native";
import { type Edge, SafeAreaView } from "react-native-safe-area-context";

import { useTokens } from "../../theme/ThemeProvider";
import { gutter } from "../../theme/tokens";
import { useBreakpoint } from "../../theme/useBreakpoint";

export interface ScreenProps {
  children: React.ReactNode;
  /** Wraps children in a ScrollView. Set false when the body owns a BoundedList/FlatList. */
  scroll?: boolean;
  keyboardAvoiding?: boolean;
  edges?: Edge[];
  refreshControl?: ScrollViewProps["refreshControl"];
  contentContainerStyle?: ViewStyle;
  style?: ViewStyle;
}

/**
 * §3: screen gutter 16 (compact) / 24 (medium+), via useBreakpoint(). §6: safe-area,
 * bg1, optional scroll + keyboard-avoid — one instance per route.
 */
export function Screen({
  children,
  scroll = false,
  keyboardAvoiding = false,
  edges = ["top", "left", "right", "bottom"],
  refreshControl,
  contentContainerStyle,
  style,
}: ScreenProps) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const paddingHorizontal = isCompact ? gutter.compact : gutter.medium;

  const content = scroll ? (
    <ScrollView
      style={styles.flex}
      contentContainerStyle={[{ paddingHorizontal }, contentContainerStyle]}
      keyboardShouldPersistTaps="handled"
      refreshControl={refreshControl}
    >
      {children}
    </ScrollView>
  ) : (
    <View style={[styles.flex, { paddingHorizontal }, contentContainerStyle]}>{children}</View>
  );

  return (
    <SafeAreaView style={[styles.flex, { backgroundColor: tokens.bg1 }, style]} edges={edges}>
      {keyboardAvoiding ? (
        <KeyboardAvoidingView
          style={styles.flex}
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          keyboardVerticalOffset={Platform.OS === "ios" ? 8 : 0}
        >
          {content}
        </KeyboardAvoidingView>
      ) : (
        content
      )}
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
});
