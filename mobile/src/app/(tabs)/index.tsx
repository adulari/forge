// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — polled every 3s while focused via
// useSessions. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { router } from "expo-router";
import { Flame, Plus } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { SearchField } from "../../components/ds/SearchField";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { CostMetric } from "../../components/ds/CostMetric";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Screen } from "../../components/ds/Screen";
import { StatusDot } from "../../components/ds/StatusDot";
import { Skeleton } from "../../components/ds/Skeleton";
import { ApiError, type SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, shadowStyle, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

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
        <CostMetric valueUsd={totalCost} variant="bodyBold" />
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

export default function FleetScreen() {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  const { isExpanded } = useBreakpoint();
  const query = useSessions();

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
  const hasData = data.length > 0;
  const isFirstLoad = query.isLoading && !hasData;

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => <SessionCard row={item} index={index} />,
    [],
  );
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);

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
      <EmptyState
        icon={Flame}
          message={search.trim() ? "no sessions match that search" : "no live sessions — start one"}
          action={search.trim() ? <Button label="Clear search" variant="secondary" onPress={() => setSearch("")} /> : <Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />}
      />
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
      <SearchField
        value={search}
        onChangeText={setSearch}
        placeholder="Search sessions, paths, status"
        autoCapitalize="none"
        autoCorrect={false}
        containerStyle={styles.search}
      />
      {hasData ? <FleetHeader sessions={data} needsYouOnly={needsYouOnly} onToggleNeedsYou={() => setNeedsYouOnly((value) => !value)} /> : null}
      {isFirstLoad ? (
        <View style={styles.list}>
          {[0, 1, 2, 3].map((i) => (
            <FleetRowSkeleton key={i} />
          ))}
        </View>
      ) : (
        <BoundedList
          data={filteredData}
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
