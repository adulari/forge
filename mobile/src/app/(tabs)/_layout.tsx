// Bottom tabs (T2.1) — DESIGN_SYSTEM §7: Fleet · Inbox · History · Settings, styled from
// tokens. `expanded` (>=1024) is meant to swap to a MasterDetail rail — see the HANDOFF
// below for why that's deferred to T5.1. Inbox badge = count of `useSessions` waiting (same
// 3s-polled list Fleet/Inbox render, no extra fetch). Icon+pill do the Tabshift tick/
// cross-fade (DESIGN_SYSTEM §5.2) locally per tab — a fully custom sliding-indicator tab bar
// would need @react-navigation/bottom-tabs prop types that aren't part of this app's
// dependency surface, so this stays inside expo-router's public Tabs API
// (tabBarIcon/tabBarBadge/tabBarStyle).
import { Tabs } from "expo-router";
import { BellDot, Flame, History, Settings2, type LucideIcon } from "lucide-react-native";
import React, { useEffect } from "react";
import { StyleSheet, View } from "react-native";
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
const InboxTabIcon = makeTabIcon(BellDot);
const HistoryTabIcon = makeTabIcon(History);
const SettingsTabIcon = makeTabIcon(Settings2);

function TabsNavigator() {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const waitingCount = sessions?.filter((s) => s.waiting).length ?? 0;

  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: tokens.bg2,
          borderTopColor: tokens.border,
          borderTopWidth: StyleSheet.hairlineWidth,
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
  // HANDOFF(T2.1 -> T5.1): `expanded` (>=1024, DESIGN_SYSTEM §7) is supposed to swap this
  // for a persistent `MasterDetail` rail (fleet list + Inbox filter pills + New Session at
  // top; History/Settings collapse into the rail footer) with the session detail on the
  // right, and the bottom tab bar disappearing entirely. Wiring that up needs the Fleet
  // (T2.3) and Inbox/History (T2.4) route content to compose into the rail, which isn't
  // built yet in this task's scope — so `expanded` intentionally falls through to the same
  // tab navigator as compact/medium for now (fully usable, just not the final desktop
  // layout). T5.1 owns swapping this in.
  return <TabsNavigator />;
}

const styles = StyleSheet.create({
  iconWrap: { alignItems: "center", justifyContent: "center", width: 40, height: 32 },
  pill: { position: "absolute", width: 40, height: 32 },
  underline: { position: "absolute", top: -8, alignSelf: "center", width: 24, height: 2, borderRadius: 1 },
});
