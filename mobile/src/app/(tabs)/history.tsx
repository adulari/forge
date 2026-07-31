// History — past-session browser + resurrection (FEATURES.md §1.1, §4). Infinite/
// cursor scroll over usePastSessions() (`before` = last row's last_activity),
// client-side search filter over title/cwd, tap-to-resume via useCreateSession.
import { TabPager } from "../../components/TabPager";
import { router } from "expo-router";
import { History as HistoryIcon, MoreHorizontal } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { stripLeadingAttachMentions } from "../../components/chat/MessageRow";
import { SyncGlyph } from "../../components/anywhere/SyncGlyph";
import { Badge } from "../../components/ds/Badge";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { EmptyState } from "../../components/ds/EmptyState";
import { RelativeTime } from "../../components/ds/RelativeTime";
import { Screen } from "../../components/ds/Screen";
import { SearchField } from "../../components/ds/SearchField";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Segmented } from "../../components/ds/Segmented";
import { Skeleton } from "../../components/ds/Skeleton";
import { useToast } from "../../components/ds/ToastHost";
import { SessionLifecycleSheet } from "../../components/session/SessionLifecycleSheet";
import { ApiError, type PastSessionRow, type SessionSearchResult } from "../../lib/api";
import { useAnywhere } from "../../lib/anywhere/store";
import { syncGlyph } from "../../lib/anywhere/format";
import { useCreateSession, usePastSessions, useSessionSearch } from "../../lib/queries";
import { useForgeline, useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type } from "../../theme/typography";

interface HistoryDisplayRow extends PastSessionRow {
  running: boolean;
  match_source?: SessionSearchResult["match_source"];
  match_excerpt?: string | null;
}

function matchesQuery(row: HistoryDisplayRow, query: string): boolean {
  if (!query) return true;
  const haystack =
    `${row.id} ${row.title} ${row.cwd} ${row.match_excerpt ?? ""} ${row.preview ?? ""}`.toLowerCase();
  return haystack.includes(query);
}

export type ActivityBucket = "today" | "yesterday" | "week" | "earlier";

export function bucketForActivity(nowSec: number, lastActivitySec: number): ActivityBucket {
  const now = new Date(nowSec * 1000);
  const activity = new Date(lastActivitySec * 1000);
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  if (activity >= todayStart) return "today";

  const yesterdayStart = new Date(todayStart);
  yesterdayStart.setDate(yesterdayStart.getDate() - 1);
  if (activity >= yesterdayStart) return "yesterday";

  const weekStart = new Date(todayStart);
  const daysSinceMonday = (weekStart.getDay() + 6) % 7;
  weekStart.setDate(weekStart.getDate() - daysSinceMonday);
  return activity >= weekStart ? "week" : "earlier";
}

type HistoryFilter = "all" | "archived" | "active";
type HistoryListItem =
  | { type: "header"; bucket: ActivityBucket; label: string }
  | { type: "row"; row: HistoryDisplayRow; index: number };

const FILTERS: { value: HistoryFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "active", label: "Unarchived" },
  { value: "archived", label: "Archived" },
];

const BUCKETS: { value: ActivityBucket; label: string }[] = [
  { value: "today", label: "Today" },
  { value: "yesterday", label: "Yesterday" },
  { value: "week", label: "This week" },
  { value: "earlier", label: "Earlier" },
];

function matchesFilter(row: HistoryDisplayRow, filter: HistoryFilter): boolean {
  if (filter === "all") return true;
  return filter === "archived" ? row.archived : !row.archived;
}

interface HistoryRowProps {
  row: HistoryDisplayRow;
  index: number;
  onPress: (row: HistoryDisplayRow) => void;
  onActions: (row: HistoryDisplayRow) => void;
}

function HistoryRowBase({ row, index, onPress, onActions }: HistoryRowProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const entrance = useForgeline(index);
  // Forge Anywhere: zero visual change for signed-out users — sync meta only renders
  // once an Anywhere account exists at all.
  const { signedIn: anywhereSignedIn } = useAnywhere();
  const title = row.title || `#${row.id.slice(0, 8)}`;
  const resumeRow = useCallback(() => onPress(row), [onPress, row]);

  // Forge Anywhere: no relay sync has happened for any real session yet, so every row
  // is honestly rendered via SyncGlyph's "offline-cache" kind (glyph "◌") — the closest
  // existing SyncStatus for "local only, nothing has round-tripped through the relay".
  // Deriving the card's border tint from `syncGlyph(...).colorKey` (rather than a
  // hardcoded neutral) means once real per-row sync events land (synced/uploading/
  // conflict/etc.), a "⑂ conflict kept" row automatically gets the design's warn-tinted
  // border — the same vocabulary switch the mock uses — without inventing that state now.
  const syncStatus = anywhereSignedIn ? ({ kind: "offline-cache", cachedAt: row.last_activity * 1000 } as const) : null;
  const syncInfo = syncStatus ? syncGlyph(syncStatus) : null;
  const tintedBorder = syncInfo && (syncInfo.colorKey === "warn" || syncInfo.colorKey === "danger");
  const borderColor = tintedBorder ? hexToRgba(tokens[syncInfo!.colorKey], 0.3) : tokens.border;

  return (
    <Animated.View style={entrance}>
      <Animated.View style={[strike.style, styles.wrap]}>
        {/* Keep Resume and Archive as sibling controls. Nesting the archive Pressable inside the
            row Pressable renders <button><button /></button> on web and breaks hydration. */}
        <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor }]}>
          <Pressable
            onPress={resumeRow}
            onPressIn={strike.onPressIn}
            onPressOut={strike.onPressOut}
            accessibilityRole="button"
            accessibilityLabel={`Resume ${title}`}
          >
            <View style={styles.inner}>
              <View style={styles.headerRow}>
                <Text style={[type.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
                  {title}
                </Text>
                {row.running ? <Badge label="running" tone="success" /> : null}
                {row.archived ? <Badge label="archived" tone="neutral" /> : null}
              </View>
              <Text
                style={[type.sub, styles.cwd, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
                numberOfLines={1}
                ellipsizeMode="head"
              >
                {row.cwd}
              </Text>
              {row.match_excerpt || row.preview ? (
                <Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={2}>
                  {stripLeadingAttachMentions(row.match_excerpt || row.preview || "")}
                </Text>
              ) : null}
              <View style={[styles.footerRow, styles.footerWithActions]}>
                <RelativeTime timestampMs={row.last_activity * 1000} />
                <View style={styles.metaRight}>
                  <Text style={[type.meta, styles.mono, { color: tokens.ink3 }, tabularNums]}>{row.message_count} msgs</Text>
                  {row.cost_usd > 0 ? <Text style={[type.meta, styles.mono, { color: tokens.success }, tabularNums]}>{formatCost(row.cost_usd)}</Text> : null}
                  {syncStatus ? <SyncGlyph status={syncStatus} /> : null}
                </View>
              </View>
            </View>
          </Pressable>
          <Pressable
            style={styles.actionsButton}
            onPress={() => onActions(row)}
            accessibilityRole="button"
            accessibilityLabel={`Actions for ${title}`}
            hitSlop={space.space8}
          >
            <MoreHorizontal size={17} strokeWidth={1.75} color={tokens.ink3} />
          </Pressable>
        </View>
      </Animated.View>
    </Animated.View>
  );
}

const HistoryRow = React.memo(HistoryRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.onPress === next.onPress &&
    prev.onActions === next.onActions &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.archived === b.archived &&
    a.message_count === b.message_count &&
    a.cost_usd === b.cost_usd &&
    a.preview === b.preview &&
    a.last_activity === b.last_activity &&
    a.running === b.running &&
    a.match_excerpt === b.match_excerpt
  );
});

export function HistoryScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<HistoryFilter>("all");
  const [confirmRow, setConfirmRow] = useState<HistoryDisplayRow | null>(null);
  const [actionsRow, setActionsRow] = useState<HistoryDisplayRow | null>(null);
  const [resumingId, setResumingId] = useState<string | null>(null);
  const [nowSec] = useState(() => Math.floor(Date.now() / 1000));

  const {
    data,
    isLoading,
    isError,
    error,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    refetch,
    isRefetching,
  } = usePastSessions();
  const globalSearch = useSessionSearch(query, 100);
  const createSession = useCreateSession();

  const normalizedQuery = query.trim().toLowerCase();
  const serverSearching = normalizedQuery.length >= 2;
  const rows = useMemo<HistoryDisplayRow[]>(
    () =>
      serverSearching
        ? (globalSearch.data ?? []).map((row) => ({
            id: row.id,
            title: row.title,
            cwd: row.cwd,
            worktree: null,
            archived: row.archived,
            running: row.running,
            message_count: row.message_count,
            cost_usd: row.cost_usd,
            last_activity: row.last_activity,
            created_at: row.last_activity,
            preview: row.match_excerpt,
            match_source: row.match_source,
            match_excerpt: row.match_excerpt,
          }))
        : (data?.pages.flat() ?? []).map((row) => ({ ...row, running: false })),
    [data, globalSearch.data, serverSearching],
  );
  const filteredRows = useMemo(
    () => rows.filter((row) => matchesQuery(row, normalizedQuery) && matchesFilter(row, filter)),
    [rows, normalizedQuery, filter],
  );
  const listItems = useMemo<HistoryListItem[]>(() => {
    const groups = new Map<ActivityBucket, HistoryDisplayRow[]>();
    for (const row of filteredRows) {
      const bucket = bucketForActivity(nowSec, row.last_activity);
      const group = groups.get(bucket) ?? [];
      group.push(row);
      groups.set(bucket, group);
    }

    return BUCKETS.flatMap(({ value, label }) => {
      const group = groups.get(value);
      if (!group?.length) return [];
      return [
        { type: "header" as const, bucket: value, label },
        ...group.map((row, index) => ({ type: "row" as const, row, index })),
      ];
    });
  }, [filteredRows, nowSec]);

  const resume = useCallback(
    (row: HistoryDisplayRow) => {
      setResumingId(row.id);
      createSession.mutate(
        { resume: row.id },
        {
          onSuccess: (created) => {
            setResumingId(null);
            router.push(`/session/${created.id}`);
          },
          onError: (err) => {
            setResumingId(null);
            toast.show(err instanceof ApiError ? err.message : "could not resume session.", {
              tone: "danger",
            });
          },
        },
      );
    },
    [createSession, toast],
  );

  const onRowPress = useCallback((row: HistoryDisplayRow) => {
    if (row.running) {
      router.push(`/session/${row.id}`);
      return;
    }
    setConfirmRow(row);
  }, []);
  const onActions = useCallback((row: HistoryDisplayRow) => setActionsRow(row), []);

  const renderItem = useCallback(
    ({ item }: { item: HistoryListItem }) =>
      item.type === "header"
        ? <SectionHeader>{item.label}</SectionHeader>
        : (
            <HistoryRow
              row={item.row}
              index={item.index}
              onPress={onRowPress}
              onActions={onActions}
            />
          ),
    [onActions, onRowPress],
  );
  const keyExtractor = useCallback(
    (item: HistoryListItem) => (item.type === "header" ? `header:${item.bucket}` : item.row.id),
    [],
  );

  const onEndReached = useCallback(() => {
    if (!serverSearching && hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [fetchNextPage, hasNextPage, isFetchingNextPage, serverSearching]);

  const visibleLoading =
    serverSearching
      ? globalSearch.isDebouncing || (globalSearch.isFetching && globalSearch.data == null)
      : isLoading;
  const visibleError = serverSearching ? globalSearch.error : error;
  const visibleIsError = serverSearching ? globalSearch.isError : isError;
  const visibleRefetch = serverSearching ? globalSearch.refetch : refetch;
  const visibleIsRefetching = serverSearching
    ? globalSearch.isFetching && !globalSearch.isDebouncing
    : isRefetching;

  return (
    <Screen scroll={false} contentContainerStyle={styles.screenPad}>
      <SearchField
        value={query}
        onChangeText={setQuery}
        placeholder="search every session and message…"
        autoCapitalize="none"
        autoCorrect={false}
        containerStyle={styles.search}
      />
      <View style={styles.filters}>
        <Segmented options={FILTERS} value={filter} onChange={setFilter} testID="history-filter" />
      </View>
      {/* Forge Anywhere: design calls for a sync-state banner here (retrying/offline/
          storage-full/key-update) sourced from useAnywhere().account.syncBanner, storage-
          full linking to /anywhere/storage. AnywhereAccount (lib/anywhere/types.ts) has no
          `syncBanner` field yet, so this is intentionally omitted rather than fabricated —
          wire it in once the foundation type grows that field. */}
      {visibleLoading ? (
        <View>
          {[0, 1, 2].map((i) => (
            <View key={i} style={styles.skeletonRow}>
              <Skeleton width="55%" height={17} />
              <Skeleton width="70%" height={12} style={styles.skeletonGap} />
              <Skeleton width="40%" height={12} style={styles.skeletonGap} />
            </View>
          ))}
        </View>
      ) : (
        <BoundedList
          data={listItems}
          keyExtractor={keyExtractor}
          renderItem={renderItem}
          ListEmptyComponent={
            visibleIsError ? (
              <EmptyState
                icon={HistoryIcon}
                message={visibleError instanceof ApiError ? visibleError.message : "something's wrong — couldn't load history."}
                action={<Button label="Retry" variant="secondary" onPress={() => visibleRefetch()} />}
              />
            ) : (
              <EmptyState
                icon={HistoryIcon}
                message={normalizedQuery || filter !== "all" ? "no past sessions match these filters." : "no past sessions yet."}
              />
            )
          }
          refreshing={visibleIsRefetching}
          onRefresh={visibleRefetch}
          onEndReached={onEndReached}
          loadingMore={!serverSearching && isFetchingNextPage}
          contentContainerStyle={styles.listPad}
        />
      )}
      <ConfirmDialog
        visible={confirmRow != null}
        title="Resume this session?"
        message={confirmRow?.title || confirmRow?.id.slice(0, 8)}
        confirmLabel="Resume"
        onConfirm={() => {
          if (confirmRow) resume(confirmRow);
          setConfirmRow(null);
        }}
        onCancel={() => setConfirmRow(null)}
      />
      <SessionLifecycleSheet
        target={
          actionsRow
            ? {
                id: actionsRow.id,
                title: actionsRow.title,
                cwd: actionsRow.cwd,
                archived: actionsRow.archived,
                running: actionsRow.running,
              }
            : null
        }
        visible={actionsRow != null}
        onClose={() => setActionsRow(null)}
      />
      {resumingId ? (
        <View style={[StyleSheet.absoluteFill, styles.resumeOverlay, { backgroundColor: tokens.overlayScrim }]} accessibilityViewIsModal accessibilityRole="alert" accessibilityLabel="Resuming session">
          <ActivityIndicator color={tokens.accent} />
          <Text style={[type.body, { color: tokens.ink }]}>Resuming session…</Text>
        </View>
      ) : null}
    </Screen>
  );
}

const styles = StyleSheet.create({
  screenPad: { paddingTop: space.space12 },
  search: { marginBottom: space.space8 },
  // A horizontal ScrollView stretches on its cross-axis inside a flex column on
  // History "empty gap" note no longer applies (the horizontal Chip ScrollView is gone,
  // replaced by a full-width Segmented control) — kept as the bottom-margin wrapper.
  filters: { paddingBottom: space.space8 },
  listPad: { paddingBottom: space.space32 },
  // Machined card row (gap-separated, bordered) — replaces the old hairline-separated
  // de-boxed row so History matches the same bordered-card list treatment as Fleet.
  wrap: { paddingHorizontal: space.space16, paddingTop: space.space4, paddingBottom: space.space8 },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, overflow: "hidden" },
  inner: {
    paddingHorizontal: space.space16,
    paddingVertical: space.space16,
    gap: space.space8,
  },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  cwd: {},
  footerRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  footerWithActions: { paddingRight: space.space32 },
  metaRight: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  mono: { fontFamily: monoFamily.regular },
  actionsButton: { position: "absolute", right: space.space16, bottom: space.space16 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonGap: { marginTop: space.space8 },
  resumeOverlay: { alignItems: "center", justifyContent: "center" },
});

export default function HistoryTab() {
  return (
    <TabPager index={2}>
      <HistoryScreen />
    </TabPager>
  );
}
