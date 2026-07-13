// Inbox — sessions waiting on a human (FEATURES.md §4: "waiting is the killer signal";
// the server already sorts waiting sessions first, `useSessions()` just filters them
// down). DecisionPeek (T4.3) mounts a "peek" affordance per row so a waiting session's
// PermissionCard/QuestionCard can be answered without leaving the Inbox; the row itself
// still navigates into the full session on tap.
import { router } from "expo-router";
import { CircleCheck, Eye } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { Badge } from "../../components/ds/Badge";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { EmptyState } from "../../components/ds/EmptyState";
import { HeatEdge } from "../../components/ds/HeatEdge";
import { IconButton } from "../../components/ds/IconButton";
import { RelativeTime } from "../../components/ds/RelativeTime";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { StatusDot } from "../../components/ds/StatusDot";
import { ApiError, type SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useForgeline, useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

interface InboxRowProps {
  row: SessionRow;
  index: number;
  onPress: (row: SessionRow) => void;
  onPeek: (row: SessionRow) => void;
}

function InboxRowBase({ row, index, onPress, onPeek }: InboxRowProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const entrance = useForgeline(index);
  const title = row.title || `#${row.id.slice(0, 8)}`;
  const rowRef = useRef<React.ComponentRef<typeof Pressable>>(null);

  const onRowPress = useCallback(() => onPress(row), [onPress, row]);

  // The row's trailing "Peek" IconButton is a real nested <button>, so on react-native-web the
  // row itself can't also be an actual <button> (accessibilityRole="button" renders one) — that's
  // an invalid button-in-button and breaks hydration. Keep the row a plain focusable <div> on
  // web and replicate Space-to-activate manually; Enter already works unconditionally via RNW's
  // press responder. Native (iOS/Android) keeps accessibilityRole="button" as-is.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = rowRef.current as unknown as HTMLElement | null;
    if (!node) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === " " || e.key === "Spacebar") {
        e.preventDefault();
        onRowPress();
      }
    };
    node.addEventListener("keydown", onKeyDown);
    return () => node.removeEventListener("keydown", onKeyDown);
  }, [onRowPress]);

  return (
    <Animated.View style={entrance}>
      <Animated.View style={strike.style}>
        <Pressable
          ref={rowRef}
          onPress={onRowPress}
          onPressIn={strike.onPressIn}
          onPressOut={strike.onPressOut}
          accessibilityRole={Platform.OS === "web" ? undefined : "button"}
          accessibilityLabel={`${title}, needs you`}
        >
          {/* DESIGN_ELEVATION.md Move 2 — de-boxed row: every Inbox row is waiting, so
              every row carries the selection wash + heat edge (no per-row card). */}
          <View style={[styles.rowBg, { backgroundColor: tokens.selection }]}>
            <HeatEdge state="waiting" />
            <View style={styles.inner}>
              <View style={styles.headerRow}>
                <StatusDot state="waiting" />
                <Text style={[type.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
                  {title}
                </Text>
                <Badge label="NEEDS YOU" tone="danger" shape="pill" />
              </View>
              <Text
                style={[type.sub, styles.cwd, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
                numberOfLines={1}
                ellipsizeMode="head"
              >
                {row.cwd}
              </Text>
              <View style={styles.footerRow}>
                <RelativeTime timestampMs={row.last_activity * 1000} />
                <View style={styles.footerSpacer} />
                <IconButton
                  icon={<Eye size={16} strokeWidth={1.75} color={tokens.ink2} />}
                  onPress={() => onPeek(row)}
                  accessibilityLabel={`Peek at ${title}`}
                  style={styles.peekButton}
                />
              </View>
            </View>
          </View>
        </Pressable>
      </Animated.View>
      <View style={[styles.separator, { backgroundColor: tokens.border }]} />
    </Animated.View>
  );
}

const InboxRow = React.memo(InboxRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.onPress === next.onPress &&
    prev.onPeek === next.onPeek &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.last_activity === b.last_activity
  );
});

export default function InboxScreen() {
  const { isExpanded } = useBreakpoint();
  const query = useSessions();
  const rows = useMemo(() => (query.data ?? []).filter((s) => s.waiting), [query.data]);
  const [peekSessionId, setPeekSessionId] = useState<string | null>(null);

  const onRowPress = useCallback((row: SessionRow) => {
    router.push(`/session/${row.id}`);
  }, []);
  const onRowPeek = useCallback((row: SessionRow) => {
    setPeekSessionId(row.id);
  }, []);
  const closePeek = useCallback(() => setPeekSessionId(null), []);

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => (
      <InboxRow row={item} index={index} onPress={onRowPress} onPeek={onRowPeek} />
    ),
    [onRowPress, onRowPeek],
  );
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);

  // T5.1 (fixed): expanded's MasterDetail rail (ExpandedRail in (tabs)/_layout.tsx) already
  // renders this same waiting-filtered list via its "Waiting" pill — this screen just fills
  // the detail pane's `<Slot/>` there, so rendering the full Inbox list here too duplicated it.
  if (isExpanded) {
    if (query.isLoading) {
      return <Screen scroll={false}><View style={styles.skeletonRow}><Skeleton width="55%" height={17} /><Skeleton width="70%" height={12} /></View></Screen>;
    }
    if (query.isError) {
      return <Screen scroll={false}><EmptyState icon={CircleCheck} message={query.error instanceof ApiError ? query.error.message : "Could not load waiting sessions."} action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />} /></Screen>;
    }
    return (
      <Screen scroll={false}>
        <EmptyState icon={CircleCheck} message="select a waiting session to see it here." />
      </Screen>
    );
  }

  if (query.isLoading) {
    return (
      <Screen scroll={false} contentContainerStyle={styles.listPad}>
        {[0, 1, 2].map((i) => (
          <View key={i} style={styles.skeletonRow}>
            <Skeleton width="55%" height={17} />
            <Skeleton width="70%" height={12} style={styles.skeletonGap} />
            <Skeleton width="30%" height={12} style={styles.skeletonGap} />
          </View>
        ))}
      </Screen>
    );
  }

  const emptyComponent = query.isError ? (
    <EmptyState
      icon={CircleCheck}
      message={query.error instanceof ApiError ? query.error.message : "something's wrong — couldn't load the inbox."}
      action={<Button label="Retry" variant="secondary" onPress={() => query.refetch()} />}
    />
  ) : (
    <EmptyState icon={CircleCheck} message="nothing needs you right now." />
  );

  return (
    <Screen scroll={false}>
      <BoundedList
        data={rows}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
        refreshing={query.isRefetching}
        onRefresh={query.refetch}
        contentContainerStyle={styles.listPad}
      />
      <DecisionPeek sessionId={peekSessionId} visible={peekSessionId != null} onClose={closePeek} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  listPad: { paddingTop: space.space12, paddingBottom: space.space32 },
  rowBg: { position: "relative" },
  inner: {
    minHeight: 72,
    justifyContent: "center",
    paddingHorizontal: space.space16,
    paddingVertical: space.space16,
    gap: space.space8,
  },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  cwd: {},
  footerRow: { flexDirection: "row", alignItems: "center" },
  footerSpacer: { flex: 1 },
  peekButton: { marginVertical: -space.space12 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonGap: { marginTop: space.space8 },
});
