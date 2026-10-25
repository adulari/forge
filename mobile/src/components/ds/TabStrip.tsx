// Machined flat tab strip (Session Chat/Tasks/Agents/Review) — type-first text tabs on a
// hairline baseline: active = ink + 2px accent underline, inactive = ink2, optional mono
// count suffix or waiting dot. Distinct from Segmented (the pill control stays for true
// value pickers like temper/appearance); these are navigation tabs, never a pill/fill.
import { LinearGradient } from "expo-linear-gradient";
import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor, tabularNums } from "../../theme/typography";

export interface TabStripOption<T extends string> {
  value: T;
  label: string;
  /** Mono count rendered after the label (omitted when undefined). */
  badge?: number;
  /** Waiting/attention dot rendered after the label. */
  dot?: boolean;
}

export interface TabStripProps<T extends string> {
  options: TabStripOption<T>[];
  value: T;
  onChange: (value: T) => void;
  testID?: string;
  /** The surface the strip sits on, which its overflow fades blend into (default bg1). */
  background?: string;
}

const FADE_WIDTH = 28;

export function TabStrip<T extends string>({ options, value, onChange, testID, background }: TabStripProps<T>) {
  const tokens = useTokens();
  const surface = background ?? tokens.bg1;
  const scrollRef = useRef<ScrollView>(null);
  const tabX = useRef(new Map<T, { x: number; width: number }>());
  const [viewport, setViewport] = useState(0);
  const [content, setContent] = useState(0);
  const [offset, setOffset] = useState(0);
  // On a phone the strip is wider than the screen; a fade at the cut edge says there is more
  // instead of a tab clipped mid-word ("Re…").
  const fadeEnd = content - viewport - offset > 4;
  const fadeStart = offset > 4;

  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => setOffset(e.nativeEvent.contentOffset.x), []);
  // Keep the active tab on screen (a segment opened by shortcut or deep link, say).
  useEffect(() => {
    const tab = tabX.current.get(value);
    if (!tab || viewport === 0) return;
    if (tab.x < offset || tab.x + tab.width > offset + viewport) {
      scrollRef.current?.scrollTo({ x: Math.max(0, tab.x - FADE_WIDTH), animated: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, viewport]);

  return (
    <View style={[styles.scroller, { borderBottomColor: tokens.hairline }]}>
      <ScrollView
        ref={scrollRef}
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={styles.row}
        testID={testID}
        accessibilityRole="tablist"
        onLayout={(e: LayoutChangeEvent) => setViewport(e.nativeEvent.layout.width)}
        onContentSizeChange={(w: number) => setContent(w)}
        onScroll={onScroll}
        scrollEventThrottle={32}
      >
        {options.map((option) => {
          const active = option.value === value;
          return (
            <Pressable
              key={option.value}
              onPress={() => onChange(option.value)}
              onLayout={(e: LayoutChangeEvent) => tabX.current.set(option.value, { x: e.nativeEvent.layout.x, width: e.nativeEvent.layout.width })}
              accessibilityRole="tab"
              accessibilityState={{ selected: active }}
              accessibilityLabel={
                option.badge != null ? `${option.label}, ${option.badge}` : option.label
              }
              style={styles.tab}
              hitSlop={{ top: 8, bottom: 8 }}
            >
              <View style={styles.labelRow}>
                <Text style={[styles.label, { color: active ? tokens.ink : tokens.ink2 }]}>{option.label}</Text>
                {option.badge != null ? (
                  <Text style={[styles.badge, tabularNums, { color: tokens.ink4 }]}>{option.badge}</Text>
                ) : null}
                {option.dot ? <View style={[styles.dot, { backgroundColor: tokens.danger }]} /> : null}
              </View>
              {active ? <View style={[styles.underline, { backgroundColor: tokens.accent }]} /> : null}
            </Pressable>
          );
        })}
      </ScrollView>
      {fadeStart ? (
        <LinearGradient
          pointerEvents="none"
          colors={[surface, hexToRgba(surface, 0)]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 0 }}
          style={[styles.fade, styles.fadeLeft]}
        />
      ) : null}
      {fadeEnd ? (
        <LinearGradient
          pointerEvents="none"
          colors={[hexToRgba(surface, 0), surface]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 0 }}
          style={[styles.fade, styles.fadeRight]}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  scroller: {
    flexGrow: 0,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  row: {
    flexDirection: "row",
    gap: space.space16,
    paddingRight: space.space8,
  },
  fade: { position: "absolute", top: 0, bottom: 0, width: FADE_WIDTH },
  fadeLeft: { left: 0 },
  fadeRight: { right: 0 },
  tab: { position: "relative", paddingBottom: 10 },
  labelRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  label: { fontSize: 12.5, fontWeight: "600", letterSpacing: 0.4, fontFamily: sansFamilyFor(600) },
  badge: { fontFamily: monoFamily.regular, fontSize: 11 },
  dot: { width: 5, height: 5, borderRadius: 3 },
  underline: { position: "absolute", left: 0, right: 0, bottom: 0, height: 2 },
});
