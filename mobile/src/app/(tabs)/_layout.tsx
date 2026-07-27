// Compact navigation uses the platform tab bar on iOS so iOS 26 can render its native
// Liquid Glass material and interaction. Other compact targets keep the existing Expo
// Router tabs, while expanded layouts use the persistent root-level Fleet rail.
//
// INVARIANT — every file in this directory must be a real tab in BOTH navigators below.
// A `(tabs)` route with no `<NativeTabs.Trigger>` is not "hidden", it is DELETED: expo-router
// collects untriggered (and `hidden`-triggered) children into `protectedScreens` and
// `useSortedScreens` drops them from the navigator entirely (layouts/withLayoutContext.js,
// useScreens.js), so a NAVIGATE to one is silently unhandled on iOS while it still works on
// every other target. `<Tabs.Screen href={null}>` has no NativeTabs equivalent — `hidden` is
// stricter, and navigating to a hidden trigger additionally throws in dev / falls back to
// tab 0 in release (native-tabs/NativeBottomTabsNavigator.js). Screens reached by push
// rather than by tab (Floor, Plans) therefore live in the ROOT stack — `app/floor.tsx`,
// `app/plans.tsx` — which keeps their `/floor` and `/plans` URLs and leaves the native bar
// untouched. `src/lib/tabRoutes.test.ts` fails the build if this drifts.
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
          // Machined's tab-bar/footer band reads a shade darker than the card
          // background (`bg2`) — `bg0` is the closest existing token to the design's
          // dedicated footer neutral (undocumented 4th neutral, see INVENTORY.md
          // "Design tokens observed"). `hairline` (not `border`) matches the design's
          // exact top-rule alpha.
          backgroundColor: tokens.bg0,
          borderTopColor: tokens.hairline,
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

  // Expanded layouts navigate by the persistent Fleet rail, not a tab bar. The swipe pager lives
  // in each tab route (see components/TabPager.tsx for why it cannot wrap the navigator).
  if (isExpanded) return <Slot />;
  if (Platform.OS === "ios") return <IOSNativeTabs />;
  return <StandardTabs />;
}
