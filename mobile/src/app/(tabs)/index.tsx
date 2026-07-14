// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — polled every 3s while focused via
// useSessions. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { router } from "expo-router";
import { Flame, Plus } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { FlatList, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { SearchField } from "../../components/ds/SearchField";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { CostMetric } from "../../components/ds/CostMetric";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { StatusDot } from "../../components/ds/StatusDot";
import { ApiError, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useServerFleets, useSessions } from "../../lib/queries";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, shadowStyle, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

type FleetDeckItem = { type: "session"; row: SessionRow } | { type: "label"; label: string };

// DESIGN_ELEVATION.md Move 3 — the one identity moment: the ⚒ mark beside the Fleet title.
function FleetTitle() {
  const tokens = useTokens();
  return (
    <View style={styles.titleRow}>
      <Text style={[typeScale.title, styles.titleText, { color: tokens.ink }]}>Fleet</Text>
      <Text
        style={[styles.mark, { color: tokens.ink3 }]}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        ⚒
      </Text>
    </View>
  );
}

// DESIGN_ELEVATION.md Move 2 — airy 3-up of *type* (big tabular number + tiny uppercase
// label), hairline-separated, NOT three bordered tiles.
function FleetHeader({ sessions, needsYouOnly, onToggleNeedsYou }: { sessions: SessionRow[]; needsYouOnly: boolean; onToggleNeedsYou: () => void }) {
  const tokens = useTokens();
  const totalCost = useMemo(() => sessions.reduce((sum, s) => sum + s.cost_usd, 0), [sessions]);
  const waitingCount = useMemo(() => sessions.filter((s) => s.waiting).length, [sessions]);
  const busyCount = useMemo(() => sessions.filter((s) => s.busy).length, [sessions]);

  return (
    <View style={[styles.header, { borderBottomColor: tokens.border }]}>
      <View style={styles.headerStat}>
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>spend</Text>
        <CostMetric valueUsd={totalCost} variant="bodyBold" showZero />
      </View>
      <Pressable
        onPress={onToggleNeedsYou}
        style={[styles.headerStat, styles.headerDivider, { borderColor: tokens.border, backgroundColor: needsYouOnly ? tokens.selection : "transparent" }]}
        accessibilityRole="button"
        accessibilityLabel="Filter sessions needing a response"
      >
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>waiting</Text>
        <View style={styles.waitingCount}>
          {waitingCount > 0 ? <StatusDot state="waiting" /> : null}
          <Text style={[typeScale.bodyBold, tabularNums, { color: waitingCount > 0 ? tokens.danger : tokens.ink }]}>{waitingCount}</Text>
        </View>
      </Pressable>
      <View style={[styles.headerStat, styles.headerDivider, { borderColor: tokens.border }]}>
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>busy</Text>
        <Text style={[typeScale.bodyBold, tabularNums, { color: tokens.ink }]} numberOfLines={1}>
          {busyCount}
        </Text>
      </View>
    </View>
  );
}

function SessionJumpStrip({ sessions, onPress }: { sessions: SessionRow[]; onPress: (sessionId: string) => void }) {
  const tokens = useTokens();
  const visible = sessions.slice(0, 14);

  if (visible.length === 0) return null;

  return (
    <View style={[styles.jumpStrip, { borderBottomColor: tokens.border }]}>
      <Text style={[typeScale.section, { color: tokens.ink3 }]}>sessions</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.jumpStripDots}>
        {visible.map((session) => (
          <Pressable
            key={session.id}
            onPress={() => onPress(session.id)}
            style={({ pressed }) => [styles.jumpDot, { backgroundColor: tokens.ink3, opacity: pressed ? 0.6 : 1 }]}
            accessibilityRole="button"
            accessibilityLabel={`Jump to ${session.title || session.id}`}
          />
        ))}
        {sessions.length > visible.length ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>+{sessions.length - visible.length}</Text> : null}
      </ScrollView>
    </View>
  );
}

function FleetRowSkeleton() {
  return (
    <View style={styles.skeletonRow}>
      <View style={styles.skeletonRow1}>
        <Skeleton width={8} height={8} radius={4} />
        <Skeleton width="45%" height={17} />
      </View>
      <Skeleton width="70%" height={12} style={styles.skeletonGap} />
      <Skeleton width="40%" height={12} style={styles.skeletonGap} />
    </View>
  );
}

function FleetServerSwitcher() {
  const tokens = useTokens();
  const { servers, activeServerId, setActive } = useAuth();
  const fleets = useServerFleets(servers);

  if (servers.length <= 1) return null;

  return (
    <View style={[styles.switcher, { borderBottomColor: tokens.border }]}>
      <Text style={[typeScale.section, { color: tokens.ink3 }]}>servers</Text>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        style={styles.serverListScroll}
        contentContainerStyle={styles.serverList}
      >
        {servers.map((server, index) => {
          const fleet = fleets[index];
          const reachable = fleet.isSuccess;
          const rows: SessionRow[] = fleet.data ?? [];
          const count = rows.filter((row) => row.waiting).length;
          return (
            <Pressable
              key={server.id}
              onPress={() => setActive(server.id)}
              accessibilityRole="button"
              accessibilityLabel={`${server.name}, ${reachable ? "reachable" : "unreachable"}, ${count} waiting`}
              accessibilityState={{ selected: server.id === activeServerId }}
              style={({ pressed }) => [
                styles.serverChip,
                { backgroundColor: server.id === activeServerId ? tokens.selection : tokens.bg3, opacity: pressed ? 0.72 : 1 },
              ]}
            >
              <View style={[styles.serverDot, { backgroundColor: fleet.isPending ? tokens.warn : reachable ? tokens.success : tokens.danger }]} />
              <Text style={[typeScale.meta, { color: server.id === activeServerId ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
                {server.name}
              </Text>
              <Text style={[typeScale.meta, { color: reachable ? tokens.ink3 : tokens.ink4 }]}>{count}</Text>
            </Pressable>
          );
        })}
      </ScrollView>
    </View>
  );
}


export default function FleetScreen() {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  const { isExpanded } = useBreakpoint();
  const query = useSessions();
  const listRef = React.useRef<FlatList<FleetDeckItem>>(null);

  // `useSessions` polls every few seconds while focused (module doc above) — react-query's own
  // `isRefetching` flips true on THAT background poll too, not just a manual pull, so wiring it
  // straight into `refreshing` made the pull-to-refresh spinner fire on its own every poll tick.
  // Track only the pull-triggered refetch instead.
  const [manualRefreshing, setManualRefreshing] = useState(false);
  const [search, setSearch] = useState("");
  const [needsYouOnly, setNeedsYouOnly] = useState(false);
  const { refetch } = query;
  const onRefresh = useCallback(() => {
    setManualRefreshing(true);
    void refetch().finally(() => setManualRefreshing(false));
  }, [refetch]);

  const data = useMemo(() => query.data ?? [], [query.data]);
  const filteredData = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return data.filter((row) => {
      if (needsYouOnly && !row.waiting) return false;
      if (!needle) return true;
      const status = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
      return [row.title, row.cwd, status].some((value) => value.toLowerCase().includes(needle));
    });
  }, [data, search, needsYouOnly]);
  const deckRows = useMemo(() => {
    const rows: FleetDeckItem[] = [];
    let previous: string | null = null;
    for (const row of filteredData) {
      const group = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
      if (group !== previous) rows.push({ type: "label", label: group === "waiting" ? "NEEDS YOU" : group === "busy" ? "FORGING" : "COOL" });
      rows.push({ type: "session", row });
      previous = group;
    }
    return rows;
  }, [filteredData]);
  const hasData = data.length > 0;
  const isFirstLoad = query.isLoading && !hasData;

  const renderItem = useCallback(
    ({ item }: { item: (typeof deckRows)[number] }) => item.type === "label" ? <Text style={[typeScale.section, styles.groupLabel, { color: item.label === "NEEDS YOU" ? tokens.danger : tokens.ink3 }]}>{item.label}</Text> : <SessionCard row={item.row} index={data.indexOf(item.row)} />,
    [data, tokens.danger, tokens.ink3],
  );
  const keyExtractor = useCallback((item: (typeof deckRows)[number]) => item.type === "label" ? `label:${item.label}` : item.row.id, []);

  const emptyComponent = useMemo(() => {
    if (query.isError) {
      return (
        <EmptyState
          icon={Flame}
          message={query.error instanceof ApiError ? query.error.message : "server unreachable"}
          action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />}
        />
      );
    }
    return (
      <View style={styles.emptyWrap}>
        <View style={styles.emptyAsh} accessibilityElementsHidden>
          {[0, 1, 2, 3, 4].map((index) => <View key={index} style={[styles.ashCoal, { backgroundColor: tokens.ink4 }]} />)}
        </View>
        <EmptyState
          icon={Flame}
          message={search.trim() ? "no sessions match that search" : "no live sessions — start one"}
          action={search.trim() ? <Button label="Clear search" variant="secondary" onPress={() => setSearch("")} /> : <Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />}
        />
      </View>
    );
  }, [query.isError, query.error, search]);

  // T5.1 (fixed): expanded's MasterDetail rail already renders the live session list —
  // this screen fills the detail pane's `<Slot/>` in that layout (see (tabs)/_layout.tsx),
  // so rendering the full Fleet list here too duplicated it side by side. Selecting a
  // session pushes `session/[id]` over both panes (HANDOFF in _layout.tsx), so the detail
  // pane never actually shows this screen's list content on expanded — just the placeholder.
  if (isExpanded) {
    return (
      <Screen scroll={false}>
        <EmptyState icon={Flame} message="select a session from the fleet to see it here." />
      </Screen>
    );
  }

  return (
    <Screen scroll={false}>
      <FleetTitle />
      <FleetServerSwitcher />
      <SearchField
        value={search}
        onChangeText={setSearch}
        placeholder="Search sessions, paths, status"
        autoCapitalize="none"
        autoCorrect={false}
        containerStyle={styles.search}
      />
      {hasData ? <><FleetHeader sessions={data} needsYouOnly={needsYouOnly} onToggleNeedsYou={() => setNeedsYouOnly((value) => !value)} /><SessionJumpStrip sessions={filteredData} onPress={(sessionId) => { const target = deckRows.findIndex((item) => item.type === "session" && item.row.id === sessionId); if (target >= 0) listRef.current?.scrollToIndex({ index: target, animated: true }); }} /></> : null}
      {isFirstLoad ? (
        <View style={styles.list}>
          {[0, 1, 2, 3].map((i) => (
            <FleetRowSkeleton key={i} />
          ))}
        </View>
      ) : (
        <BoundedList
          ref={listRef}
          data={deckRows}
          keyExtractor={keyExtractor}
          renderItem={renderItem}
          ListEmptyComponent={emptyComponent}
          refreshing={manualRefreshing}
          onRefresh={onRefresh}
          contentContainerStyle={styles.listContent}
        />
      )}

      <IconButton
        icon={<Plus size={24} strokeWidth={1.75} color={tokens.onAccent} />}
        onPress={() => router.push("/new-session")}
        accessibilityLabel="New session"
        style={[
          styles.fab,
          { backgroundColor: tokens.accent },
          depth.raised ? shadowStyle(depth.raised) : null,
        ]}
      />
    </Screen>
  );
}

const FAB_SIZE = 56;

const styles = StyleSheet.create({
  switcher: { gap: space.space8, paddingTop: space.space12, paddingBottom: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  // Horizontal ScrollViews stretch on their cross-axis in a flex column on web; pin to content.
  serverListScroll: { flexGrow: 0, flexShrink: 0 },
  serverList: { gap: space.space8 },
  serverChip: { minHeight: 44, maxWidth: 220, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, borderRadius: radii.radiusPill },
  serverDot: { width: 8, height: 8, borderRadius: 4 },
  titleRow: { flexDirection: "row", alignItems: "center", paddingTop: space.space12 },
  titleText: { letterSpacing: -0.4 },
  mark: { fontSize: 14, marginLeft: space.space8 },
  list: { paddingTop: space.space12 },
  search: { paddingTop: space.space12 },
  listContent: { paddingTop: space.space12, paddingBottom: 96 },
  header: {
    flexDirection: "row",
    paddingTop: space.space16,
    paddingBottom: space.space16,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerStat: { flex: 1, gap: space.space4, alignItems: "flex-start" },
  waitingCount: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  emptyWrap: { flex: 1 },
  emptyAsh: { flexDirection: "row", justifyContent: "center", gap: space.space8, paddingTop: space.space24 },
  ashCoal: { width: 6, height: 6, borderRadius: 3 },
  jumpStrip: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  jumpStripDots: { alignItems: "center", gap: space.space8 },
  jumpDot: { width: 6, height: 6, borderRadius: 3 },
  groupLabel: { paddingTop: space.space16, paddingHorizontal: space.space16 },
  headerDivider: { borderLeftWidth: StyleSheet.hairlineWidth, paddingLeft: space.space12 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonRow1: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
  fab: {
    position: "absolute",
    right: space.space16,
    bottom: space.space24,
    width: FAB_SIZE,
    height: FAB_SIZE,
    borderRadius: radii.radiusPill,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 0,
  },
});
