import { Flame, History, Plus, Settings2 } from "lucide-react-native";
import { router, usePathname } from "expo-router";
import React, { useMemo } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { Chip } from "../ds/Chip";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { MasterDetail } from "../ds/MasterDetail";
import { SessionCard } from "./SessionCard";
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

function RailPill({ href, label, count }: { href: "/" | "/floor" | "/inbox"; label: string; count?: number }) {
  const selected = usePathname() === href;
  return <Chip label={count ? `${label} (${count})` : label} selected={selected} onPress={() => router.push(href)} />;
}

function RailFooterIcon({ href, icon, label }: { href: "/history" | "/settings"; icon: React.ReactNode; label: string }) {
  const tokens = useTokens();
  const active = usePathname() === href;
  return <IconButton icon={icon} onPress={() => router.push(href)} accessibilityLabel={label} style={active ? { backgroundColor: tokens.bg3, borderRadius: radii.radius8 } : undefined} />;
}

export function ExpandedFleetRail() {
  const tokens = useTokens();
  const pathname = usePathname();
  const { data: sessions } = useSessions();
  const rows = useMemo(() => sessions ?? [], [sessions]);
  const waitingCount = useMemo(() => rows.filter((row) => row.waiting).length, [rows]);
  const showingInbox = pathname === "/inbox";
  const visibleRows = showingInbox ? rows.filter((row) => row.waiting) : rows;

  return (
    <View style={styles.rail}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Fleet</Text>
        <IconButton icon={<Plus size={20} strokeWidth={1.75} color={tokens.accent} />} onPress={() => router.push("/new-session")} accessibilityLabel="New session" />
      </View>
      <View style={styles.pills}><RailPill href="/" label="All" /><RailPill href="/floor" label="Floor" count={rows.filter((row) => row.busy).length} /><RailPill href="/inbox" label="Waiting" count={waitingCount} /></View>
      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>{visibleRows.length === 0 ? <EmptyState icon={Flame} message={showingInbox ? "nothing needs you right now." : "no live sessions — start one"} /> : visibleRows.map((row, index) => <SessionCard key={row.id} row={row} index={index} />)}</ScrollView>
      <View style={[styles.footer, { borderTopColor: tokens.border }]}><RailFooterIcon href="/history" icon={<History size={20} strokeWidth={1.75} color={tokens.ink2} />} label="History" /><RailFooterIcon href="/settings" icon={<Settings2 size={20} strokeWidth={1.75} color={tokens.ink2} />} label="Settings" /></View>
    </View>
  );
}

export function DesktopDrillDown({ children }: { children: React.ReactNode }) {
  const { isExpanded } = useBreakpoint();
  return isExpanded ? <MasterDetail master={<ExpandedFleetRail />} detail={children} /> : children;
}

const styles = StyleSheet.create({
  rail: { flex: 1 },
  header: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingHorizontal: space.space16, paddingVertical: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  pills: { flexDirection: "row", gap: space.space8, paddingHorizontal: space.space16, paddingVertical: space.space12 },
  list: { flex: 1 },
  listContent: { paddingBottom: space.space16 },
  footer: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: space.space8, paddingVertical: space.space8, borderTopWidth: StyleSheet.hairlineWidth },
});
