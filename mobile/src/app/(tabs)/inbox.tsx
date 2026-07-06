// Inbox — sessions waiting on a human (FEATURES.md §4: "waiting is the killer signal";
// the server already sorts waiting sessions first, `useSessions()` just filters them
// down). DecisionPeek (T4.3) mounts a "peek" affordance per row so a waiting session's
// PermissionCard/QuestionCard can be answered without leaving the Inbox; the row itself
// still navigates into the full session on tap.
import { router } from "expo-router";
import { CircleCheck, Eye } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { Badge } from "../../components/ds/Badge";
import { BoundedList } from "../../components/ds/BoundedList";
import { EmptyState } from "../../components/ds/EmptyState";
import { HeatEdge } from "../../components/ds/HeatEdge";
import { IconButton } from "../../components/ds/IconButton";
import { RelativeTime } from "../../components/ds/RelativeTime";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { StatusDot } from "../../components/ds/StatusDot";
import type { SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useForgeline, useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

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

  return (
    <Animated.View style={entrance}>
      <Animated.View style={strike.style}>
        <Pressable
          onPress={() => onPress(row)}
          onPressIn={strike.onPressIn}
          onPressOut={strike.onPressOut}
          accessibilityRole="button"
          accessibilityLabel={`${title}, needs you`}
        >
          {/* DESIGN_ELEVATION.md Move 2 — de-boxed row: every Inbox row is waiting, so
              every row carries the selection wash + heat edge (no per-row card). */}
          <View style={[styles.rowBg, { backgroundColor: tokens.selection }]}>
            <HeatEdge active />
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

  return (
    <Screen scroll={false}>
      <BoundedList
        data={rows}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={<EmptyState icon={CircleCheck} message="nothing needs you right now." />}
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
