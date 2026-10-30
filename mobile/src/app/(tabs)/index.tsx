// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — refreshed by fleet events with a slow
// recovery poll. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { TabPager } from "../../components/TabPager";
import { router } from "expo-router";
import { Flame, Search } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { FlatList, Keyboard, Platform, Pressable, StyleSheet, Text, View } from "react-native";

import { DecisionCard } from "../../components/cards/DecisionCard";
import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { TaskComposer } from "../../components/ds/TaskComposer";
import { OfflineHosts } from "../../components/fleet/OfflineHosts";
import { ProjectChips } from "../../components/fleet/ProjectChips";
import { ServerScopePills } from "../../components/fleet/ServerScopePills";
import { ApiError, type SessionRow } from "../../lib/api";
import { ServerScope } from "../../lib/auth";
import { filterByProject, projectChips } from "../../lib/fleetProjects";
import { useScopedFleet } from "../../lib/useScopedFleet";
import { useUsage } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { buildFleetDeck, DONE_LABEL_PREFIX, type FleetDeckItem } from "../../lib/fleetRows";
import { filterSessions, isOfflineError, sessionPickerState } from "../../lib/sessionPicker";
import { space } from "../../theme/tokens";
import { useShortcutLabel } from "../../lib/shortcuts";
import { formatCostSummary, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
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
  // The daemon's own since-midnight spend: summing the listed sessions' lifetime cost labelled a
  // multi-day total "today" and counted plan usage as dollars. An older daemon without `today`
  // gets no figure rather than a wrong one.
  const todayCost = useUsage().data?.today?.combined.costUsd;
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
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}> · {busyCount} forging</Text>
      {todayCost != null ? (
        <>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}> · </Text>
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>{formatCostSummary(todayCost)}</Text>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}> API today</Text>
        </>
      ) : null}
    </Pressable>
  );
}

function FleetRowSkeleton() {
  return (
    <Card style={styles.skeletonCard}>
      <View style={styles.skeletonRow1}>
        <Skeleton width={8} height={8} radius={4} />
        <Skeleton width="45%" height={17} />
      </View>
      <Skeleton width="70%" height={12} style={styles.skeletonGap} />
      <Skeleton width="40%" height={12} style={styles.skeletonGap} />
    </Card>
  );
}

export function FleetScreen() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const fleet = useScopedFleet();
  const listRef = React.useRef<FlatList<FleetDeckItem>>(null);

  // `useSessions` still has a slow recovery poll in addition to fleet events. React Query's
  // `isRefetching` covers those automatic refreshes too, so track only pull-triggered refreshes
  // here and never animate the spinner without a gesture.
  const [manualRefreshing, setManualRefreshing] = useState(false);
  const [needsYouOnly, setNeedsYouOnly] = useState(false);
  const [project, setProject] = useState<string | null>(null);
  const [peek, setPeek] = useState<{ sessionId: string; serverId: string } | null>(null);
  const [composerText, setComposerText] = useState("");
  const newSessionHint = useShortcutLabel("session.new");
  // The composer is `position: absolute`, so it never participates in layout resize — neither
  // iOS (which never resizes the window for the keyboard) nor Android (Android 15+ enforces
  // edge-to-edge, so `adjustResize` no longer shrinks the window either) lifts it for free, so
  // the pill needs its own offset on both. `keyboardWillShow`/`keyboardWillHide` only exist on
  // iOS (no native equivalent on Android — RN's Keyboard.js); Android uses
  // `keyboardDidShow`/`keyboardDidHide`, which fire with real metrics independent of the window
  // resize (only API<30 depended on that).
  //
  // The offset is the keyboard's overlap with THIS screen, not its height: the keyboard also
  // covers the tab bar and the bottom inset under the screen, so lifting by the full height left
  // the pill floating a tab bar's height above the keys, over the session rows.
  const [keyboardOffset, setKeyboardOffset] = useState(0);
  const keyboardAnchor = React.useRef<View>(null);
  useEffect(() => {
    if (Platform.OS === "web") return;
    const showEvent = Platform.OS === "ios" ? "keyboardWillShow" : "keyboardDidShow";
    const hideEvent = Platform.OS === "ios" ? "keyboardWillHide" : "keyboardDidHide";
    const show = Keyboard.addListener(showEvent, (e) => {
      const anchor = keyboardAnchor.current;
      if (!anchor) {
        setKeyboardOffset(e.endCoordinates.height);
        return;
      }
      anchor.measureInWindow((_x, screenBottom) =>
        setKeyboardOffset(Math.max(0, screenBottom - e.endCoordinates.screenY)),
      );
    });
    const hide = Keyboard.addListener(hideEvent, () => setKeyboardOffset(0));
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);
  const { refetch } = fleet;
  const onRefresh = useCallback(() => {
    setManualRefreshing(true);
    void refetch().finally(() => setManualRefreshing(false));
  }, [refetch]);
  const serverByRowId = useMemo(() => new Map(fleet.rows.map((item) => [item.row.id, item.serverId])), [fleet.rows]);
  const onPeek = useCallback((row: SessionRow) => setPeek({ sessionId: row.id, serverId: serverByRowId.get(row.id) ?? "" }), [serverByRowId]);
  const closePeek = useCallback(() => setPeek(null), []);
  // Hearth core rule 6: the composer replaces the "new session" affordance everywhere —
  // it hands its typed text off to the full "Forge a task" sheet, which owns project/
  // model/permission selection.
  const onComposerSubmit = useCallback((text: string) => {
    setComposerText("");
    router.push({ pathname: "/new-session", params: { title: text } });
  }, []);

  const data = useMemo(() => fleet.rows.map((item) => item.row), [fleet.rows]);
  const chips = useMemo(() => projectChips(data), [data]);
  // A project whose last session went away stops filtering, instead of leaving an empty list.
  const activeProject = project && chips.some((chip) => chip.name === project) ? project : null;
  const filteredData = useMemo(
    () => filterByProject(filterSessions(data, "", needsYouOnly), activeProject),
    [data, needsYouOnly, activeProject],
  );
  const [showDone, setShowDone] = useState(false);
  const deckRows = useMemo(() => buildFleetDeck(filteredData, data, showDone), [filteredData, data, showDone]);
  const hasData = data.length > 0;
  const isFirstLoad = fleet.isLoading && !hasData;
  const showingLastKnown = fleet.showingLastKnown;
  const pickerState = sessionPickerState({ isLoading: isFirstLoad, isError: fleet.isError, visibleCount: filteredData.length });

  const renderItem = useCallback(
    ({ item }: { item: FleetDeckItem }) => {
      if (item.type === "label" && item.label.startsWith(DONE_LABEL_PREFIX)) {
        return (
          <Pressable
            onPress={() => setShowDone((value) => !value)}
            accessibilityRole="button"
            accessibilityState={{ expanded: showDone }}
            accessibilityLabel={`${item.label.replace(" · ", ", ")} sessions, ${showDone ? "collapse" : "expand"}`}
            hitSlop={8}
          >
            <Text style={[typeScale.section, styles.groupLabel, { color: tokens.ink4 }]}>
              {showDone ? "▾ " : "▸ "}
              {item.label}
            </Text>
          </Pressable>
        );
      }
      if (item.type === "label") {
        return <Text style={[typeScale.section, styles.groupLabel, { color: tokens.ink4 }]}>{item.label}</Text>;
      }
      const card = item.row.waiting ? (
        <DecisionCard row={item.row} index={item.sourceIndex} onPeek={onPeek} />
      ) : (
        <SessionCard row={item.row} index={item.sourceIndex} />
      );
      const serverId = serverByRowId.get(item.row.id);
      return serverId ? <ServerScope serverId={serverId}>{card}</ServerScope> : card;
    },
    [tokens.ink4, onPeek, serverByRowId, showDone],
  );
  const keyExtractor = useCallback((item: (typeof deckRows)[number]) => item.type === "label" ? `label:${item.label}` : item.row.id, []);

  const emptyComponent = useMemo(() => {
    if (fleet.isError) {
      return (
        <EmptyState
          icon={Flame}
          message={isOfflineError(fleet.error) ? "Forge is offline. Check the server connection and try again." : fleet.error instanceof ApiError ? fleet.error.message : "Unable to load sessions."}
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
  }, [fleet.isError, fleet.error, refetch, tokens.ink4]);

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
              <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>{newSessionHint}</Text>
            </View>
          </View>
        </View>
      </Screen>
    );
  }

  return (
    <Screen scroll={false} contentContainerStyle={styles.screen}>
      <FleetTitle />
      {pickerState === "ready" ? <FleetSummary sessions={data} needsYouOnly={needsYouOnly} onToggleNeedsYou={() => setNeedsYouOnly((value) => !value)} /> : null}
      <ServerScopePills />
      <ProjectChips chips={chips} selected={activeProject} onSelect={setProject} />
      {showingLastKnown ? (
        <Text style={[typeScale.monoMeta, styles.lastKnown, { color: tokens.ink4 }]} accessibilityLiveRegion="polite">
          {fleet.isError ? "Last known fleet — host not reachable" : "Last known fleet — updating…"}
        </Text>
      ) : null}
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
          ListFooterComponent={<OfflineHosts />}
          refreshing={manualRefreshing}
          onRefresh={onRefresh}
          contentContainerStyle={styles.listContent}
        />
      )}

      <View ref={keyboardAnchor} pointerEvents="none" style={styles.keyboardAnchor} />
      {/* Hearth core rule 6: bottom-floating task composer replaces the FAB — the one
          "new session" affordance on mobile Fleet. */}
      <TaskComposer
        value={composerText}
        onChangeText={setComposerText}
        onSubmit={onComposerSubmit}
        // `bottom: 0` already sits flush with the safe area here — Screen's SafeAreaView
        // (`edges` includes "bottom" by default) shrinks this View by insets.bottom itself,
        // so adding it again would double-count it. `keyboardOffset` lifts the pill clear
        // of the keyboard on iOS and Android alike (see the listener above).
        style={[styles.composer, { bottom: space.space16 + keyboardOffset }]}
        testID="fleet-composer"
      />

      <ServerScope serverId={peek?.serverId ?? ""}>
        <DecisionPeek sessionId={peek?.sessionId ?? null} visible={peek != null} onClose={closePeek} />
      </ServerScope>
    </Screen>
  );
}

const styles = StyleSheet.create({
  lastKnown: { paddingHorizontal: space.space16, paddingBottom: space.space8 },
  // Screen's own 16pt gutter is zeroed here — SessionCard/DecisionCard/the skeleton card
  // already carry their own 16pt inset, so leaving Screen's padding on doubled it to 32pt.
  // The header rows below (title/summary/chip strips) carry the single 16pt gutter directly
  // instead, so they land flush with the cards rather than the old extra 16pt step.
  screen: { paddingHorizontal: 0 },
  titleRow: { flexDirection: "row", alignItems: "center", paddingTop: space.space12, paddingHorizontal: space.space16 },
  titleText: { letterSpacing: -0.4 },
  mark: { fontSize: 14, marginLeft: space.space8, padding: space.space4 },
  titleSpacer: { flex: 1 },
  searchButton: { marginRight: -space.space12 },
  list: { paddingTop: space.space12 },
  listContent: { paddingTop: space.space12, paddingBottom: 96 },
  summary: { flexDirection: "row", flexWrap: "wrap", marginTop: space.space2, paddingHorizontal: space.space16 },
  emptyWrap: { flex: 1 },
  emptyAsh: { flexDirection: "row", justifyContent: "center", gap: space.space8, paddingTop: space.space24 },
  ashCoal: { width: 6, height: 6, borderRadius: 3 },
  groupLabel: { paddingTop: space.space16, paddingHorizontal: space.space16 },
  // Matches the real SessionCard/DecisionCard gap-separated bordered-card treatment so
  // the loading state doesn't flash a different layout than the content it's standing in for.
  skeletonCard: { marginHorizontal: space.space16, marginBottom: space.space12, gap: space.space8 },
  skeletonRow1: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
  composer: { position: "absolute", left: space.space16, right: space.space16 },
  keyboardAnchor: { position: "absolute", left: 0, right: 0, bottom: 0, height: 0 },
  expandedEmpty: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space16, paddingHorizontal: space.space24 },
  expandedEmptyMessage: { fontSize: 14, lineHeight: 20, textAlign: "center" },
  expandedComposerWrap: { position: "relative", width: 560, maxWidth: "100%" },
  expandedComposerHintWrap: { position: "absolute", right: 54, top: 0, bottom: 0, alignItems: "center", justifyContent: "center" },
});

export default function FleetTab() {
  return (
    <TabPager index={0}>
      <FleetScreen />
    </TabPager>
  );
}
