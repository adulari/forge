// Inbox — sessions waiting on a human (FEATURES.md §4: "waiting is the killer signal";
// the server already sorts waiting sessions first, `useSessions()` just filters them
// down). DecisionPeek approve-in-place (FEATURES §5, DESIGN_SYSTEM §6) is T4.3 — until
// then a row tap only navigates into the full session.
import { router } from "expo-router";
import { CircleCheck } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { Badge } from "../../components/ds/Badge";
import { BoundedList } from "../../components/ds/BoundedList";
import { Card } from "../../components/ds/Card";
import { EmptyState } from "../../components/ds/EmptyState";
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
}

function InboxRowBase({ row, index, onPress }: InboxRowProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const entrance = useForgeline(index);
  const title = row.title || `#${row.id.slice(0, 8)}`;

  return (
    <Animated.View style={entrance}>
      <Animated.View style={[strike.style, styles.cardGap]}>
        <Pressable
          onPress={() => onPress(row)}
          onPressIn={strike.onPressIn}
          onPressOut={strike.onPressOut}
          accessibilityRole="button"
          accessibilityLabel={`${title}, needs you`}
        >
          <Card>
            <View style={styles.headerRow}>
              <StatusDot state="waiting" />
              <Text style={[type.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
                {title}
              </Text>
              <Badge label="NEEDS YOU" tone="danger" shape="pill" />
            </View>
            <Text
              style={[type.codeSmall, styles.cwd, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
              numberOfLines={1}
              ellipsizeMode="head"
            >
              {row.cwd}
            </Text>
            <View style={styles.footerRow}>
              <RelativeTime timestampMs={row.last_activity * 1000} />
              {/* HANDOFF(T4.3): mount a DecisionPeek trigger here so a waiting session's
                  PermissionCard/QuestionCard can be answered without leaving the Inbox. */}
            </View>
          </Card>
        </Pressable>
      </Animated.View>
    </Animated.View>
  );
}

const InboxRow = React.memo(InboxRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.onPress === next.onPress &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.last_activity === b.last_activity
  );
});

export default function InboxScreen() {
  const query = useSessions();
  const rows = useMemo(() => (query.data ?? []).filter((s) => s.waiting), [query.data]);

  const onRowPress = useCallback((row: SessionRow) => {
    router.push(`/session/${row.id}`);
  }, []);

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => (
      <InboxRow row={item} index={index} onPress={onRowPress} />
    ),
    [onRowPress],
  );
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);

  if (query.isLoading) {
    return (
      <Screen scroll={false} contentContainerStyle={styles.listPad}>
        {[0, 1, 2].map((i) => (
          <Card key={i} style={styles.cardGap}>
            <Skeleton width="55%" height={17} />
            <Skeleton width="70%" height={12} style={styles.skeletonGap} />
            <Skeleton width="30%" height={12} style={styles.skeletonGap} />
          </Card>
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
    </Screen>
  );
}

const styles = StyleSheet.create({
  listPad: { paddingTop: space.space12, paddingBottom: space.space32 },
  cardGap: { marginBottom: space.space8 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  cwd: { marginTop: space.space4 },
  footerRow: { flexDirection: "row", alignItems: "center", marginTop: space.space8 },
  skeletonGap: { marginTop: space.space8 },
});
