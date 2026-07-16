// Bottom tabs (T2.1) — DESIGN_SYSTEM §7: Fleet · Inbox · History · Settings, styled from
// tokens. `expanded` (>=1024) uses the persistent root-level Fleet rail. Inbox badge = count
// of `useSessions` waiting (same
// 3s-polled list Fleet/Inbox render, no extra fetch). Icon+pill do the Tabshift tick/
// cross-fade (DESIGN_SYSTEM §5.2) locally per tab — a fully custom sliding-indicator tab bar
// would need @react-navigation/bottom-tabs prop types that aren't part of this app's
// dependency surface, so this stays inside expo-router's public Tabs API
// (tabBarIcon/tabBarBadge/tabBarStyle).
import { Slot, Tabs } from "expo-router";
import { BellDot, Flame, History, PanelsTopLeft, Settings2, type LucideIcon } from "lucide-react-native";
import React, { useEffect } from "react";
import { Platform, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSequence,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import { useSessions } from "../../lib/queries";
import { durations, easings, springs } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { useBreakpoint } from "../../theme/useBreakpoint";

interface TabIconProps {
  focused: boolean;
  color: import("react-native").ColorValue;
  size?: number;
}

function makeTabIcon(Icon: LucideIcon) {
  return function TabIcon({ focused, color }: TabIconProps) {
    const tokens = useTokens();
    const reduced = useReducedMotion();
    const scale = useSharedValue(1);
    const pillOpacity = useSharedValue(focused ? 1 : 0);

    useEffect(() => {
      pillOpacity.value = reduced
        ? focused
          ? 1
          : 0
        : withTiming(focused ? 1 : 0, { duration: durations.fast, easing: easings.standard });
      if (focused && !reduced) {
        scale.value = withSequence(withSpring(1.04, springs.press), withSpring(1, springs.press));
      }
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [focused, reduced]);

    const pillStyle = useAnimatedStyle(() => ({ opacity: pillOpacity.value }));
    const iconStyle = useAnimatedStyle(() => ({ transform: [{ scale: scale.value }] }));

    return (
      <View style={styles.iconWrap}>
        {/* DESIGN_ELEVATION.md Move 3 — active tab's 2px ember heat underline. */}
        <Animated.View style={[styles.underline, pillStyle, { backgroundColor: tokens.accent }]} />
        <Animated.View
          style={[styles.pill, pillStyle, { backgroundColor: tokens.bg3, borderRadius: radii.radius12 }]}
        />
        <Animated.View style={iconStyle}>
          <Icon size={22} color={color} strokeWidth={1.75} />
        </Animated.View>
      </View>
    );
  };
}

const FleetTabIcon = makeTabIcon(Flame);
const FloorTabIcon = makeTabIcon(PanelsTopLeft);
const InboxTabIcon = makeTabIcon(BellDot);
const HistoryTabIcon = makeTabIcon(History);
const SettingsTabIcon = makeTabIcon(Settings2);

function TabsNavigator() {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const waitingCount = sessions?.filter((s) => s.waiting).length ?? 0;
  // Web: react-navigation's default tab bar height doesn't account for `env(safe-area-inset-bottom)`
  // the way its native counterpart does — without this, the home-indicator/PWA-standalone inset
  // eats into the label row and clips it (reported as "Settings" -> "Settinas"). `viewport-fit=cover`
  // (+html.tsx) is what makes `useSafeAreaInsets()` read a non-zero value here on web at all.
  // Native is untouched: react-navigation already reserves the inset there.
  const insets = useSafeAreaInsets();
  const webTabBar = Platform.OS === "web" ? { height: 58 + insets.bottom, paddingBottom: insets.bottom } : null;

  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: tokens.bg2,
          borderTopColor: tokens.border,
          borderTopWidth: StyleSheet.hairlineWidth,
          ...webTabBar,
        },
        tabBarActiveTintColor: tokens.accent,
        tabBarInactiveTintColor: tokens.ink3,
        tabBarLabelStyle: { fontSize: 11, fontWeight: "600" },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Fleet",
          tabBarIcon: FleetTabIcon,
          tabBarAccessibilityLabel: "Fleet",
        }}
      />
      <Tabs.Screen
        name="floor"
        options={{
          title: "Floor",
          tabBarIcon: FloorTabIcon,
          tabBarAccessibilityLabel: "Floor",
          tabBarBadge: sessions?.filter((s) => s.busy).length || undefined,
          tabBarBadgeStyle: { backgroundColor: tokens.accent, color: tokens.onAccent },
        }}
      />
      <Tabs.Screen
        name="inbox"
        options={{
          title: "Inbox",
          tabBarIcon: InboxTabIcon,
          tabBarAccessibilityLabel: "Inbox",
          tabBarBadge: waitingCount > 0 ? waitingCount : undefined,
          tabBarBadgeStyle: { backgroundColor: tokens.danger, color: tokens.onAccent },
        }}
      />
      <Tabs.Screen
        name="history"
        options={{
          title: "History",
          tabBarIcon: HistoryTabIcon,
          tabBarAccessibilityLabel: "History",
        }}
      />
      <Tabs.Screen
        name="settings"
        options={{
          title: "Settings",
          tabBarIcon: SettingsTabIcon,
          tabBarAccessibilityLabel: "Settings",
        }}
      />
    </Tabs>
  );
}

export default function TabsLayout() {
  const { isExpanded } = useBreakpoint();

  // The paired root layout owns the persistent expanded rail so it remains
  // visible while sibling routes such as session/[id] render in the detail pane.
  if (isExpanded) {
    return <Slot />;
  }
  return <TabsNavigator />;
}

const styles = StyleSheet.create({
  iconWrap: { alignItems: "center", justifyContent: "center", width: 40, height: 32 },
  pill: { position: "absolute", width: 40, height: 32 },
  underline: { position: "absolute", top: -8, alignSelf: "center", width: 24, height: 2, borderRadius: 1 },
});
