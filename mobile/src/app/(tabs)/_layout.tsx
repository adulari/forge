// Compact navigation uses the platform tab bar on iOS so iOS 26 can render its native
// Liquid Glass material and interaction. Other compact targets keep the existing Expo
// Router tabs, while expanded layouts use the persistent root-level Fleet rail.
import { Slot, Tabs } from "expo-router";
import { NativeTabs } from "expo-router/unstable-native-tabs";
import { BellDot, Flame, History, Settings2, type LucideIcon } from "lucide-react-native";
import React from "react";
import { Platform, StyleSheet } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { useBreakpoint } from "../../theme/useBreakpoint";

interface TabIconProps {
  color: import("react-native").ColorValue;
  size?: number;
}

function makeTabIcon(Icon: LucideIcon) {
  return function TabIcon({ color }: TabIconProps) {
    return <Icon size={22} color={color} strokeWidth={1.75} />;
  };
}

const FleetTabIcon = makeTabIcon(Flame);
const InboxTabIcon = makeTabIcon(BellDot);
const HistoryTabIcon = makeTabIcon(History);
const SettingsTabIcon = makeTabIcon(Settings2);

function IOSNativeTabs() {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const waitingCount = sessions?.filter((session) => session.waiting).length ?? 0;

  return (
    <NativeTabs
      backgroundColor="transparent"
      blurEffect="systemDefault"
      shadowColor="transparent"
      minimizeBehavior="automatic"
      iconColor={{ default: tokens.ink3, selected: tokens.accent }}
      labelStyle={{
        default: { color: tokens.ink3, fontSize: 11, fontWeight: "500" },
        selected: { color: tokens.accent, fontSize: 11, fontWeight: "600" },
      }}
      badgeBackgroundColor={tokens.danger}
    >
      <NativeTabs.Trigger name="index">
        <NativeTabs.Trigger.Icon sf={{ default: "flame", selected: "flame.fill" }} />
        <NativeTabs.Trigger.Label>Fleet</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="inbox">
        <NativeTabs.Trigger.Icon sf={{ default: "bell", selected: "bell.fill" }} />
        <NativeTabs.Trigger.Label>Inbox</NativeTabs.Trigger.Label>
        <NativeTabs.Trigger.Badge hidden={waitingCount === 0}>•</NativeTabs.Trigger.Badge>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="history">
        <NativeTabs.Trigger.Icon sf={{ default: "clock", selected: "clock.fill" }} />
        <NativeTabs.Trigger.Label>History</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="settings">
        <NativeTabs.Trigger.Icon sf={{ default: "gearshape", selected: "gearshape.fill" }} />
        <NativeTabs.Trigger.Label>Settings</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}

function StandardTabs() {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const waitingCount = sessions?.filter((session) => session.waiting).length ?? 0;
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
        options={{ title: "Fleet", tabBarIcon: FleetTabIcon, tabBarAccessibilityLabel: "Fleet" }}
      />
      <Tabs.Screen name="floor" options={{ href: null }} />
      <Tabs.Screen name="plans" options={{ href: null }} />
      <Tabs.Screen
        name="inbox"
        options={{
          title: "Inbox",
          tabBarIcon: InboxTabIcon,
          tabBarBadge: waitingCount > 0 ? "" : undefined,
          tabBarBadgeStyle: { backgroundColor: tokens.danger },
          tabBarAccessibilityLabel: waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox",
        }}
      />
      <Tabs.Screen
        name="history"
        options={{ title: "History", tabBarIcon: HistoryTabIcon, tabBarAccessibilityLabel: "History" }}
      />
      <Tabs.Screen
        name="settings"
        options={{ title: "Settings", tabBarIcon: SettingsTabIcon, tabBarAccessibilityLabel: "Settings" }}
      />
    </Tabs>
  );
}

export default function TabsLayout() {
  const { isExpanded } = useBreakpoint();

  if (isExpanded) return <Slot />;
  if (Platform.OS === "ios") return <IOSNativeTabs />;
  return <StandardTabs />;
}
