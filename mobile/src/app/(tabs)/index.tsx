// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — refreshed by fleet events with a slow
// recovery poll. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { router } from "expo-router";
import { Flame, Search } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { FlatList, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { DecisionCard } from "../../components/cards/DecisionCard";
import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { TaskComposer } from "../../components/ds/TaskComposer";
import { ApiError, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useServerFleets, useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { buildFleetDeck, type FleetDeckItem } from "../../lib/fleetRows";
import { filterSessions, isOfflineError, sessionPickerState } from "../../lib/sessionPicker";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

// DESIGN_ELEVATION.md Move 3 — the one identity moment: the ⚒ mark beside the Fleet
// title. Hearth: Floor left the tab bar, so this mark is now the primary way there.
// Hearth: the old full-width "Search sessions, paths, status" SearchField + "SESSIONS"
// jump strip are gone (HANDOFF Fleet screen has neither) — a single small search icon in
// the header routes to History, which already covers session search.
function FleetTitle() {
  const tokens = useTokens();
  return (
    <View style={styles.titleRow}>
      <Text style={[typeScale.title, styles.titleText, { color: tokens.ink }]}>Fleet</Text>
      <Pressable
        onPress={() => router.push("/floor")}
        hitSlop={space.space16}
        accessibilityRole="button"
        accessibilityLabel="Open the floor"
      >
        <Text style={[styles.mark, { color: tokens.ink3 }]}>⚒</Text>
      </Pressable>
      <View style={styles.titleSpacer} />
      <IconButton
        icon={<Search size={18} strokeWidth={1.75} color={tokens.ink3} />}
        onPress={() => router.push("/history")}
        accessibilityLabel="Search sessions"
        style={styles.searchButton}
      />
    </View>
  );
}

// Hearth: a single glanceable summary line — "N needs you · N forging · $X today" —
// replaces the old boxed 3-up stat row (HANDOFF Fleet screen).
function FleetSummary({ sessions, needsYouOnly, onToggleNeedsYou }: { sessions: SessionRow[]; needsYouOnly: boolean; onToggleNeedsYou: () => void }) {
  const tokens = useTokens();
  const totalCost = useMemo(() => sessions.reduce((sum, s) => sum + s.cost_usd, 0), [sessions]);
  const waitingCount = useMemo(() => sessions.filter((s) => s.waiting).length, [sessions]);
  const busyCount = useMemo(() => sessions.filter((s) => s.busy).length, [sessions]);

  return (
    <Pressable
      onPress={onToggleNeedsYou}
      style={styles.summary}
      accessibilityRole="button"
      accessibilityLabel="Filter sessions needing a response"
      accessibilityState={{ selected: needsYouOnly }}
    >
      <Text style={[typeScale.sub, { color: waitingCount > 0 ? tokens.danger : tokens.ink3 }]}>{waitingCount} needs you</Text>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}> · {busyCount} forging · </Text>
      <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>{formatCost(totalCost)}</Text>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}> today</Text>
    </Pressable>
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

// Hearth Fleet screen's server chips: a row of pill chips directly under the summary
// line (HANDOFF "server chips"), no section label/border — same switching behavior.
function FleetServerSwitcher() {
  const tokens = useTokens();
  const { servers, activeServerId, setActive } = useAuth();
  const fleets = useServerFleets(servers);

  // Hearth "server chips" (HANDOFF Fleet screen): always shown, even for a single
  // server — a single server just renders as a single chip.
  if (servers.length === 0) return null;

  return (
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
        const active = server.id === activeServerId;
        return (
          <Pressable
            key={server.id}
            onPress={() => setActive(server.id)}
            accessibilityRole="button"
            accessibilityLabel={`${server.name}, ${reachable ? "reachable" : "unreachable"}, ${count} waiting`}
            accessibilityState={{ selected: active }}
            // Chip stays visually compact (28pt, matching the prototype) — hitSlop
            // brings the actual touch target up to the 44pt minimum.
            hitSlop={space.space8}
            style={({ pressed }) => [
              styles.serverChip,
              { backgroundColor: active ? tokens.selection : tokens.bg3, opacity: pressed ? 0.72 : 1 },
            ]}
          >
            <View style={[styles.serverDot, { backgroundColor: fleet.isPending ? tokens.warn : reachable ? tokens.success : tokens.danger }]} />
            <Text style={[typeScale.meta, { color: active ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
              {server.name}
            </Text>
            <Text style={[typeScale.monoMeta, tabularNums, { color: reachable ? tokens.ink3 : tokens.ink4, fontFamily: monoFamily.regular }]}>{count}</Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}


export default function FleetScreen() {
  const tokens = useTokens();
  const insets = useSafeAreaInsets();
  const { isExpanded } = useBreakpoint();
  const query = useSessions();
  const listRef = React.useRef<FlatList<FleetDeckItem>>(null);

  // `useSessions` still has a slow recovery poll in addition to fleet events. React Query's
  // `isRefetching` covers those automatic refreshes too, so track only pull-triggered refreshes
  // here and never animate the spinner without a gesture.
  const [manualRefreshing, setManualRefreshing] = useState(false);
  const [needsYouOnly, setNeedsYouOnly] = useState(false);
  const [peekSessionId, setPeekSessionId] = useState<string | null>(null);
  const [composerText, setComposerText] = useState("");
  const { refetch } = query;
  const onRefresh = useCallback(() => {
    setManualRefreshing(true);
    void refetch().finally(() => setManualRefreshing(false));
  }, [refetch]);
  const onPeek = useCallback((row: SessionRow) => setPeekSessionId(row.id), []);
  const closePeek = useCallback(() => setPeekSessionId(null), []);
  // Hearth core rule 6: the composer replaces the "new session" affordance everywhere —
  // it hands its typed text off to the full "Forge a task" sheet, which owns project/
  // model/permission selection.
  const onComposerSubmit = useCallback((text: string) => {
    setComposerText("");
    router.push({ pathname: "/new-session", params: { title: text } });
  }, []);

  const data = useMemo(() => query.data ?? [], [query.data]);
  const filteredData = useMemo(() => filterSessions(data, "", needsYouOnly), [data, needsYouOnly]);
  const deckRows = useMemo(() => buildFleetDeck(filteredData, data), [filteredData, data]);
  const hasData = data.length > 0;
  const isFirstLoad = query.isLoading && !hasData;
  const pickerState = sessionPickerState({ isLoading: isFirstLoad, isError: query.isError, visibleCount: filteredData.length });

  const renderItem = useCallback(
    ({ item }: { item: FleetDeckItem }) => {
      if (item.type === "label") {
        return <Text style={[typeScale.section, styles.groupLabel, { color: tokens.ink4 }]}>{item.label}</Text>;
      }
      return item.row.waiting ? (
        <DecisionCard row={item.row} index={item.sourceIndex} onPeek={onPeek} />
      ) : (
        <SessionCard row={item.row} index={item.sourceIndex} />
      );
    },
    [tokens.ink4, onPeek],
  );
  const keyExtractor = useCallback((item: (typeof deckRows)[number]) => item.type === "label" ? `label:${item.label}` : item.row.id, []);

  const emptyComponent = useMemo(() => {
    if (query.isError) {
      return (
        <EmptyState
          icon={Flame}
          message={isOfflineError(query.error) ? "Forge is offline. Check the server connection and try again." : query.error instanceof ApiError ? query.error.message : "Unable to load sessions."}
          action={<Button label="Retry" variant="secondary" onPress={() => void refetch()} accessibilityLabel="Retry loading sessions" />}
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
          message="No sessions yet. Start with a working directory."
          action={<Button label="Create your first session" variant="primary" onPress={() => router.push("/new-session")} accessibilityLabel="Create your first session" />}
        />
      </View>
    );
  }, [query.isError, query.error, refetch, tokens.ink4]);

  // Hearth "Fleet" (web.dc.html:82-93): the expanded detail pane's empty state when no
  // session is selected — centered flame + copy + a composer pill (⌘N hint), not the old
  // generic EmptyState placeholder. Selecting a session pushes `session/[id]` over both
  // panes (HANDOFF in _layout.tsx), so this pane only ever shows the empty state.
  if (isExpanded) {
    return (
      <Screen scroll={false}>
        <View style={styles.expandedEmpty}>
          <Flame size={34} color={tokens.borderStrong} strokeWidth={1.5} />
          <Text style={[styles.expandedEmptyMessage, { color: tokens.ink3 }]}>
            Pick a session from the fleet — or forge a new one
          </Text>
          <View style={styles.expandedComposerWrap}>
            <TaskComposer
              value={composerText}
              onChangeText={setComposerText}
              onSubmit={onComposerSubmit}
              testID="fleet-empty-composer"
            />
            <View style={styles.expandedComposerHintWrap} pointerEvents="none">
              <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>⌘N</Text>
            </View>
          </View>
        </View>
      </Screen>
    );
  }

  return (
    <Screen scroll={false}>
      <FleetTitle />
      {pickerState === "ready" ? <FleetSummary sessions={data} needsYouOnly={needsYouOnly} onToggleNeedsYou={() => setNeedsYouOnly((value) => !value)} /> : null}
      <FleetServerSwitcher />
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

      {/* Hearth core rule 6: bottom-floating task composer replaces the FAB — the one
          "new session" affordance on mobile Fleet. */}
      <TaskComposer
        value={composerText}
        onChangeText={setComposerText}
        onSubmit={onComposerSubmit}
        style={[styles.composer, { bottom: space.space16 + insets.bottom }]}
        testID="fleet-composer"
      />

      <DecisionPeek sessionId={peekSessionId} visible={peekSessionId != null} onClose={closePeek} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  // Horizontal ScrollViews stretch on their cross-axis in a flex column on web; pin to content.
  serverListScroll: { flexGrow: 0, flexShrink: 0 },
  serverList: { gap: space.space8, paddingTop: space.space12 },
  serverChip: { minHeight: 28, maxWidth: 220, flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space12, borderRadius: radii.radiusPill },
  serverDot: { width: 6, height: 6, borderRadius: 3 },
  titleRow: { flexDirection: "row", alignItems: "center", paddingTop: space.space12 },
  titleText: { letterSpacing: -0.4 },
  mark: { fontSize: 14, marginLeft: space.space8, padding: space.space4 },
  titleSpacer: { flex: 1 },
  searchButton: { marginRight: -space.space12 },
  list: { paddingTop: space.space12 },
  listContent: { paddingTop: space.space12, paddingBottom: 96 },
  summary: { flexDirection: "row", flexWrap: "wrap", marginTop: space.space2 },
  emptyWrap: { flex: 1 },
  emptyAsh: { flexDirection: "row", justifyContent: "center", gap: space.space8, paddingTop: space.space24 },
  ashCoal: { width: 6, height: 6, borderRadius: 3 },
  groupLabel: { paddingTop: space.space16, paddingHorizontal: space.space16 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonRow1: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
  composer: { position: "absolute", left: space.space16, right: space.space16 },
  expandedEmpty: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space16, paddingHorizontal: space.space24 },
  expandedEmptyMessage: { fontSize: 14, lineHeight: 20, textAlign: "center" },
  expandedComposerWrap: { position: "relative", width: 560, maxWidth: "100%" },
  expandedComposerHintWrap: { position: "absolute", right: 54, top: 0, bottom: 0, alignItems: "center", justifyContent: "center" },
});
