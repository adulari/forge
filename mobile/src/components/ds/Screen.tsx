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
import { LinearGradient } from "expo-linear-gradient";

import { useTokens } from "../../theme/ThemeProvider";
import { gutter, type ColorTokens } from "../../theme/tokens";
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
 * bg1, optional scroll + keyboard-avoid — one instance per route.
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
    <SafeAreaView style={[styles.flex, { backgroundColor: tokens.bg1 }, style]} edges={edges}>
      <ForgeWash tokens={tokens} />
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

/**
 * DESIGN_ELEVATION.md Move 1 — the ONE subtle top ambient ember wash, implemented
 * once here (never per-card). Web: real CSS radial-gradient via `forgeWash`.
 * Native: a top-anchored `expo-linear-gradient` approximation (radial gradients
 * aren't supported natively), tinted with the theme's accent at the same low
 * alpha the token spec calls for (5% dark / 4% light).
 */
function ForgeWash({ tokens }: { tokens: ColorTokens }) {
  if (Platform.OS === "web") {
    return (
      <View
        style={[
          StyleSheet.absoluteFill,
          { backgroundImage: tokens.forgeWash, pointerEvents: "none" } as object,
        ]}
      />
    );
  }

  return (
    <LinearGradient
      pointerEvents="none"
      colors={[tokens.accent, tokens.accentTransparent]}
      start={{ x: 0.5, y: 0 }}
      end={{ x: 0.5, y: 1 }}
      style={[styles.wash, { opacity: tokens.forgeWashOpacity }]}
    />
  );
}

// Web-only: stops this scroll surface's rubber-band from chaining into a page-level
// bounce (RN has no typed `overscrollBehavior`, RN-web passes unknown style keys through
// to the underlying DOM node — same escape hatch `forgeWash`'s `backgroundImage` uses above).
const webScrollContain = { overscrollBehavior: "contain" };

const styles = StyleSheet.create({
  flex: { flex: 1 },
  wash: {
    position: "absolute",
    top: -80,
    left: "-20%",
    right: "-20%",
    height: 420,
  },
});
