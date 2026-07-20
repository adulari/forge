// Bottom tabs (Hearth) — 4 tabs: Fleet · Inbox · History · Settings, styled from tokens.
// Floor is no longer a tab (reached via the ⚒ mark on Fleet) but stays routable via
// `href: null`. `expanded` (>=1024) uses the persistent root-level Fleet rail. Inbox's
// badge is an unread DOT (never a count, per HANDOFF) driven off the same `useSessions`
// list Fleet/Inbox render — no extra fetch. Icon+pill do the Tabshift tick/cross-fade
// (DESIGN_SYSTEM §5.2) locally per tab — a fully custom sliding-indicator tab bar
// would need @react-navigation/bottom-tabs prop types that aren't part of this app's
// dependency surface, so this stays inside expo-router's public Tabs API
// (tabBarIcon/tabBarStyle).
import { Slot, Tabs } from "expo-router";
import { BellDot, Flame, History, Settings2, type LucideIcon } from "lucide-react-native";
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
          style={[styles.pill, pillStyle, { backgroundColor: tokens.bg3, borderColor: tokens.border, borderWidth: StyleSheet.hairlineWidth }]}
        />
        <Animated.View style={iconStyle}>
          <Icon size={22} color={color} strokeWidth={1.75} />
        </Animated.View>
      </View>
    );
  };
}

const FleetTabIcon = makeTabIcon(Flame);
const HistoryTabIcon = makeTabIcon(History);
const SettingsTabIcon = makeTabIcon(Settings2);
const BaseInboxTabIcon = makeTabIcon(BellDot);

/** Hearth: Inbox tab badge is an unread DOT, never a count (HANDOFF "Screens & navigation"). */
function InboxTabIcon(props: TabIconProps) {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const hasWaiting = (sessions ?? []).some((s) => s.waiting);
  return (
    <View>
      <BaseInboxTabIcon {...props} />
      {hasWaiting ? (
        <View
          style={[styles.dot, { backgroundColor: tokens.danger, borderColor: tokens.bg2 }]}
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        />
      ) : null}
    </View>
  );
}

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
  const tabBarBottom = insets.bottom + 10;
  const webTabBar = Platform.OS === "web" ? { height: 64, paddingBottom: 0 } : null;

  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          position: "absolute",
          left: 16,
          right: 16,
          bottom: tabBarBottom,
          height: 64,
          paddingTop: 6,
          paddingBottom: 6,
          backgroundColor: "transparent",
          borderTopWidth: 0,
          borderRadius: radii.radiusPill,
          ...webTabBar,
        },
        tabBarBackground: () => (
          <View
            pointerEvents="none"
            style={[
              StyleSheet.absoluteFill,
              styles.glassBackground,
              { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong, shadowColor: tokens.bg0 },
            ]}
          />
        ),
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
      {/* Hearth: Floor leaves the tab bar (reached via the ⚒ mark on Fleet) but stays a
          routable screen — `href: null` keeps expo-router from auto-adding a tab for it. */}
      <Tabs.Screen name="floor" options={{ href: null }} />
      {/* Plans moved into (tabs) (redesign) but stays reached via Settings, not a 5th tab. */}
      <Tabs.Screen name="plans" options={{ href: null }} />
      <Tabs.Screen
        name="inbox"
        options={{
          title: "Inbox",
          tabBarIcon: InboxTabIcon,
          tabBarAccessibilityLabel: waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox",
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
  glassBackground: {
    borderRadius: radii.radiusPill,
    borderWidth: StyleSheet.hairlineWidth,
    opacity: 0.94,
    shadowColor: "#000000",
    shadowOpacity: 0.18,
    shadowRadius: 18,
    shadowOffset: { width: 0, height: 6 },
    elevation: 12,
  },
  pill: { position: "absolute", width: 40, height: 32, borderRadius: radii.radiusPill },
  underline: { position: "absolute", bottom: -1, alignSelf: "center", width: 18, height: 2, borderRadius: 1 },
  dot: {
    position: "absolute",
    top: 3,
    right: 4,
    width: 8,
    height: 8,
    borderRadius: 4,
    borderWidth: 2,
  },
});
