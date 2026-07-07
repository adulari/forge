// DESIGN_SYSTEM.md §6 Containers — ListRow: 56pt min, Strike, hairline separator
// (inset 16), leading/trailing slots.
import React, { useEffect, useRef, useState } from "react";
import { type AccessibilityRole, Platform, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { useStrike } from "../../theme/motion";
import { rowHeight, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface ListRowProps {
  title: string;
  subtitle?: string;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
  onPress?: () => void;
  disabled?: boolean;
  /** Hairline separator inset 16 from the left, full to the right edge. Default true. */
  showSeparator?: boolean;
  accessibilityLabel?: string;
  accessibilityRole?: AccessibilityRole;
  /**
   * Set when `trailing` renders its own interactive control (e.g. a trailing IconButton).
   * On react-native-web, `accessibilityRole="button"` renders an actual `<button>` — a
   * `<button>` cannot legally contain another `<button>`, which breaks hydration. When true,
   * the row's web element stays a plain focusable `<div>` instead (Enter/Space still trigger
   * `onPress`); native (iOS/Android) behavior is unaffected either way.
   */
  hasInteractiveTrailing?: boolean;
}

export function ListRow({
  title,
  subtitle,
  leading,
  trailing,
  onPress,
  disabled = false,
  showSeparator = true,
  accessibilityLabel,
  accessibilityRole,
  hasInteractiveTrailing = false,
}: ListRowProps) {
  const tokens = useTokens();
  const { style: strikeStyle, onPressIn, onPressOut } = useStrike();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const pressableRef = useRef<React.ComponentRef<typeof Pressable>>(null);

  const suppressWebButtonTag = Platform.OS === "web" && hasInteractiveTrailing;

  // react-native-web only wires Space-to-activate for elements whose *DOM* role/tag is
  // button-ish. Once we drop the role to keep the tag a <div> (above), Space needs its own
  // handler — Enter already works unconditionally via RNW's press responder.
  useEffect(() => {
    if (!suppressWebButtonTag || !onPress || disabled) return;
    const node = pressableRef.current as unknown as HTMLElement | null;
    if (!node) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === " " || e.key === "Spacebar") {
        e.preventDefault();
        onPress();
      }
    };
    node.addEventListener("keydown", onKeyDown);
    return () => node.removeEventListener("keydown", onKeyDown);
  }, [suppressWebButtonTag, onPress, disabled]);

  const content = (
    <Animated.View
      style={[
        styles.row,
        onPress ? strikeStyle : undefined,
        onPress && hovered && !disabled ? { backgroundColor: tokens.bg3 } : undefined,
      ]}
    >
      {leading ? <View style={styles.slot}>{leading}</View> : null}
      <View style={styles.body}>
        <Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={1}>
            {subtitle}
          </Text>
        ) : null}
      </View>
      {trailing ? <View style={styles.slot}>{trailing}</View> : null}
    </Animated.View>
  );

  return (
    <View style={disabled ? styles.disabled : undefined}>
      {onPress ? (
        <Pressable
          ref={pressableRef}
          onPress={onPress}
          onPressIn={onPressIn}
          onPressOut={onPressOut}
          onHoverIn={() => setHovered(true)}
          onHoverOut={() => setHovered(false)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          disabled={disabled}
          accessibilityRole={suppressWebButtonTag ? undefined : (accessibilityRole ?? "button")}
          accessibilityLabel={accessibilityLabel ?? title}
          accessibilityState={{ disabled }}
          style={{ borderWidth: 2, borderColor: focused ? tokens.accent : "transparent" }}
        >
          {content}
        </Pressable>
      ) : (
        content
      )}
      {showSeparator ? (
        <View style={[styles.separator, { backgroundColor: tokens.border }]} />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  row: {
    minHeight: rowHeight.list,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space16,
    gap: space.space12,
  },
  slot: { alignItems: "center", justifyContent: "center" },
  body: { flex: 1, justifyContent: "center", gap: 2 },
  disabled: { opacity: 0.4 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
});
