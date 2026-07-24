// DESIGN_SYSTEM.md §6 Containers — Screen: safe-area, bg0, gutter, optional
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
  /** Distance from the true screen top to this Screen instance (e.g. a session shell's header
   * block) — RN's `KeyboardAvoidingView` docs call for this explicitly on iOS "padding" mode;
   * it is not inferred from layout automatically. Defaults to 8 (bare screen, nothing above). */
  keyboardVerticalOffset?: number;
  edges?: Edge[];
  refreshControl?: ScrollViewProps["refreshControl"];
  contentContainerStyle?: ViewStyle;
  style?: ViewStyle;
}

/**
 * §3: screen gutter 16 (compact) / 24 (medium+), via useBreakpoint(). §6: safe-area,
 * bg0, optional scroll + keyboard-avoid — one instance per route. Machined drops the
 * old ambient "forge wash" gradient entirely (thermal identity retired) — the screen
 * is just its flat background color now.
 */
export function Screen({
  children,
  scroll = false,
  keyboardAvoiding = false,
  keyboardVerticalOffset = Platform.OS === "ios" ? 8 : 0,
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
      style={[styles.flex, Platform.OS === "web" && (webScrollContain as object)]}
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
    <SafeAreaView style={[styles.flex, { backgroundColor: tokens.bg0 }, style]} edges={edges}>
      {keyboardAvoiding ? (
        <KeyboardAvoidingView
          style={styles.flex}
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          keyboardVerticalOffset={keyboardVerticalOffset}
        >
          {content}
        </KeyboardAvoidingView>
      ) : (
        content
      )}
    </SafeAreaView>
  );
}

// Web-only: stops this scroll surface's rubber-band from chaining into a page-level
// bounce (RN has no typed `overscrollBehavior`, RN-web passes unknown style keys through
// to the underlying DOM node).
const webScrollContain = { overscrollBehavior: "contain" };

const styles = StyleSheet.create({
  flex: { flex: 1 },
});
