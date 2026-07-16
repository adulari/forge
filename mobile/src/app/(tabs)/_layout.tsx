// Bottom tabs (T2.1) — DESIGN_SYSTEM §7: Fleet · Inbox · History · Settings, styled from
// tokens. `expanded` (>=1024) is meant to swap to a MasterDetail rail — see the HANDOFF
// below for why that's deferred to T5.1. Inbox badge = count of `useSessions` waiting (same
// 3s-polled list Fleet/Inbox render, no extra fetch). Icon+pill do the Tabshift tick/
// cross-fade (DESIGN_SYSTEM §5.2) locally per tab — a fully custom sliding-indicator tab bar
// would need @react-navigation/bottom-tabs prop types that aren't part of this app's
// dependency surface, so this stays inside expo-router's public Tabs API
// (tabBarIcon/tabBarBadge/tabBarStyle).
import { router, Slot, Tabs, usePathname } from "expo-router";
import { BellDot, Flame, History, PanelsTopLeft, Plus, Settings2, type LucideIcon } from "lucide-react-native";
import React, { useEffect, useMemo } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSequence,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import { Chip } from "../../components/ds/Chip";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { SearchField } from "../../components/ds/SearchField";
import { MasterDetail } from "../../components/ds/MasterDetail";
import { SessionCard } from "../../components/fleet/SessionCard";
import { useSessions } from "../../lib/queries";
import { durations, easings, springs } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
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

// ---------------------------------------------------------------------------
// Expanded (>=1024, DESIGN_SYSTEM §7) — MasterDetail rail. Rail = Fleet list +
// Inbox filter pills + New Session; History/Settings collapse into the rail
// footer (Fleet/Inbox aren't separate footer icons — the rail itself IS the
// Fleet view, and the pills ARE the Inbox filter). `detail` renders whichever
// (tabs) child route is active via `Slot`, so index/inbox/history/settings
// stay the exact same route files (ARCHITECTURE.md: "route files stay
// identical; expo-router renders the same screens into either layout").
// ---------------------------------------------------------------------------

function RailPill({ href, label, count }: { href: "/" | "/floor" | "/inbox"; label: string; count?: number }) {
  const pathname = usePathname();
  const selected = pathname === href;
  return (
    <Chip
      label={count ? `${label} (${count})` : label}
      selected={selected}
      onPress={() => router.push(href)}
    />
  );
}

function RailFooterIcon({
  href,
  icon,
  label,
}: {
  href: "/history" | "/settings";
  icon: React.ReactNode;
  label: string;
}) {
  const tokens = useTokens();
  const pathname = usePathname();
  const active = pathname === href;
  return (
    <IconButton
      icon={icon}
      onPress={() => router.push(href)}
      accessibilityLabel={label}
      style={active ? { backgroundColor: tokens.bg3, borderRadius: radii.radius8 } : undefined}
    />
  );
}

function ExpandedRail() {
  const tokens = useTokens();
  const pathname = usePathname();
  const { data: sessions } = useSessions();

  const rows = useMemo(() => sessions ?? [], [sessions]);
  const waitingCount = useMemo(() => rows.filter((s) => s.waiting).length, [rows]);
  const showingInbox = pathname === "/inbox";
  const [search, setSearch] = React.useState("");
  const visibleRows = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return (showingInbox ? rows.filter((s) => s.waiting) : rows).filter((row) => !needle || [row.title, row.cwd, row.waiting ? "waiting" : row.busy ? "busy" : "idle"].some((value) => value.toLowerCase().includes(needle)));
  }, [rows, search, showingInbox]);

  return (
    <View style={styles.rail}>
      <View style={[styles.railHeader, { borderBottomColor: tokens.border }]}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Fleet</Text>
        <IconButton
          icon={<Plus size={20} strokeWidth={1.75} color={tokens.accent} />}
          onPress={() => router.push("/new-session")}
          accessibilityLabel="New session"
        />
      </View>

      <View style={styles.pillsRow}>
        <RailPill href="/" label="All" />
        <RailPill href="/floor" label="Floor" count={rows.filter((row) => row.busy).length} />
        <RailPill href="/inbox" label="Waiting" count={waitingCount} />
      </View>

      <SearchField value={search} onChangeText={setSearch} placeholder="Search sessions" accessibilityLabel="Search sessions" containerStyle={styles.railSearch} />
      <ScrollView style={styles.railList} contentContainerStyle={styles.railListContent}>
        {visibleRows.length === 0 ? (
          <EmptyState
            icon={Flame}
            message={showingInbox ? "Nothing needs you right now." : search ? "No sessions match this search." : "No sessions yet. Start one."}
            action={!showingInbox && !search ? <Chip label="Create your first session" selected onPress={() => router.push("/new-session")} /> : search ? <Chip label="Clear search" onPress={() => setSearch("")} /> : undefined}
          />
        ) : (
          visibleRows.map((row, i) => <SessionCard key={row.id} row={row} index={i} />)
        )}
      </ScrollView>

      <View style={[styles.railFooter, { borderTopColor: tokens.border }]}>
        <RailFooterIcon
          href="/history"
          icon={<History size={20} strokeWidth={1.75} color={tokens.ink2} />}
          label="History"
        />
        <RailFooterIcon
          href="/settings"
          icon={<Settings2 size={20} strokeWidth={1.75} color={tokens.ink2} />}
          label="Settings"
        />
      </View>
    </View>
  );
}

export default function TabsLayout() {
  const { isExpanded } = useBreakpoint();

  // T5.1: expanded swaps the bottom tab bar for the persistent rail above.
  // HANDOFF(T5.1 -> future work): `session/[id]` is a sibling Stack route
  // outside this (tabs) group (see src/app/_layout.tsx's root Stack), so
  // opening a session still pushes over the whole rail+detail pair here —
  // same as compact/medium — rather than rendering inline in the detail pane
  // next to a persistent rail. True inline session embedding on expanded
  // would need `session/[id]` nested under (tabs) or `MasterDetail` lifted to
  // the root Stack; both are routing-architecture changes out of this task's
  // bounded file scope (this task only wires `(tabs)/_layout.tsx`).
  if (isExpanded) {
    return <MasterDetail master={<ExpandedRail />} detail={<Slot />} />;
  }
  return <TabsNavigator />;
}

const styles = StyleSheet.create({
  iconWrap: { alignItems: "center", justifyContent: "center", width: 40, height: 32 },
  pill: { position: "absolute", width: 40, height: 32 },
  underline: { position: "absolute", top: -8, alignSelf: "center", width: 24, height: 2, borderRadius: 1 },
  rail: { flex: 1 },
  railHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  pillsRow: { flexDirection: "row", gap: space.space8, paddingHorizontal: space.space16, paddingVertical: space.space12 },
  railSearch: { marginHorizontal: space.space16, marginBottom: space.space8 },
  railList: { flex: 1 },
  railListContent: { paddingBottom: space.space16 },
  railFooter: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    paddingVertical: space.space8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
});
