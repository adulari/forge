// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — refreshed by fleet events with a slow
// recovery poll. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { router } from "expo-router";
import { Flame } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { FlatList, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { SearchField } from "../../components/ds/SearchField";
import { DecisionCard } from "../../components/cards/DecisionCard";
import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { EmptyState } from "../../components/ds/EmptyState";
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

// Hearth Fleet screen's server chips: a row of pill chips directly under the summary
// line (HANDOFF "server chips"), no section label/border — same switching behavior.
function FleetServerSwitcher() {
  const tokens = useTokens();
  const { servers, activeServerId, setActive } = useAuth();
  const fleets = useServerFleets(servers);

  if (servers.length <= 1) return null;

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
  const [search, setSearch] = useState("");
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
  const filteredData = useMemo(() => filterSessions(data, search, needsYouOnly), [data, search, needsYouOnly]);
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
          message={search.trim() ? "No sessions match this search." : "No sessions yet. Start with a working directory."}
          action={search.trim() ? <Button label="Clear search" variant="secondary" onPress={() => setSearch("")} accessibilityLabel="Clear session search" /> : <Button label="Create your first session" variant="primary" onPress={() => router.push("/new-session")} accessibilityLabel="Create your first session" />}
        />
      </View>
    );
  }, [query.isError, query.error, refetch, search, tokens.ink4]);

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
      {pickerState === "ready" ? <FleetSummary sessions={data} needsYouOnly={needsYouOnly} onToggleNeedsYou={() => setNeedsYouOnly((value) => !value)} /> : null}
      <FleetServerSwitcher />
      <SearchField
        value={search}
        onChangeText={setSearch}
        placeholder="Search sessions, paths, status"
        autoCapitalize="none"
        autoCorrect={false}
        containerStyle={styles.search}
      />
      {pickerState === "ready" ? <SessionJumpStrip sessions={filteredData} onPress={(sessionId) => { const target = deckRows.findIndex((item) => item.type === "session" && item.row.id === sessionId); if (target >= 0) listRef.current?.scrollToIndex({ index: target, animated: true }); }} /> : null}
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
  list: { paddingTop: space.space12 },
  search: { paddingTop: space.space12 },
  listContent: { paddingTop: space.space12, paddingBottom: 96 },
  summary: { flexDirection: "row", flexWrap: "wrap", marginTop: space.space2 },
  emptyWrap: { flex: 1 },
  emptyAsh: { flexDirection: "row", justifyContent: "center", gap: space.space8, paddingTop: space.space24 },
  ashCoal: { width: 6, height: 6, borderRadius: 3 },
  jumpStrip: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  jumpStripDots: { alignItems: "center", gap: space.space8 },
  jumpDot: { width: 6, height: 6, borderRadius: 3 },
  groupLabel: { paddingTop: space.space16, paddingHorizontal: space.space16 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonRow1: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
  composer: { position: "absolute", left: space.space16, right: space.space16 },
});
