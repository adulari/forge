// Fleet — the app home + most-used surface (FEATURES.md §5 "Live fleet dashboard header").
// Aggregate header (Σ cost / waiting / busy) + live list, waiting-first because the SERVER
// already sorts it that way (never re-sort here) — polled every 3s while focused via
// useSessions. Forgeline entrance is a first-mount-only side effect of stable row keys +
// stable indices across polls (see theme/motion.ts useForgeline) — nothing extra to wire here.
import { router } from "expo-router";
import { Flame, Plus } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { CostMetric } from "../../components/ds/CostMetric";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { ApiError, type SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";

function FleetHeader({ sessions }: { sessions: SessionRow[] }) {
  const tokens = useTokens();
  const totalCost = useMemo(() => sessions.reduce((sum, s) => sum + s.cost_usd, 0), [sessions]);
  const waitingCount = useMemo(() => sessions.filter((s) => s.waiting).length, [sessions]);
  const busyCount = useMemo(() => sessions.filter((s) => s.busy).length, [sessions]);

  return (
    <Card style={styles.header}>
      <View style={styles.headerStat}>
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>spend</Text>
        <CostMetric valueUsd={totalCost} variant="bodyBold" />
      </View>
      <View style={[styles.headerStat, styles.headerDivider, { borderColor: tokens.border }]}>
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>waiting</Text>
        <Text
          style={[typeScale.bodyBold, tabularNums, { color: waitingCount > 0 ? tokens.danger : tokens.ink }]}
          numberOfLines={1}
        >
          {waitingCount}
        </Text>
      </View>
      <View style={[styles.headerStat, styles.headerDivider, { borderColor: tokens.border }]}>
        <Text style={[typeScale.section, { color: tokens.ink3 }]}>busy</Text>
        <Text style={[typeScale.bodyBold, tabularNums, { color: tokens.ink }]} numberOfLines={1}>
          {busyCount}
        </Text>
      </View>
    </Card>
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

export default function FleetScreen() {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  const query = useSessions();

  const data = useMemo(() => query.data ?? [], [query.data]);
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
        />
      );
    }
    return (
      <EmptyState
        icon={Flame}
        message="no live sessions — start one"
        action={<Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />}
      />
    );
  }, [query.isError, query.error]);

  return (
    <Screen scroll={false}>
      {isFirstLoad ? (
        <View style={styles.list}>
          {[0, 1, 2, 3].map((i) => (
            <FleetRowSkeleton key={i} />
          ))}
        </View>
      ) : (
        <BoundedList
          data={data}
          keyExtractor={keyExtractor}
          renderItem={renderItem}
          ListHeaderComponent={hasData ? <FleetHeader sessions={data} /> : undefined}
          ListEmptyComponent={emptyComponent}
          refreshing={query.isRefetching && !query.isLoading}
          onRefresh={query.refetch}
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
          depth.raised
            ? {
                shadowColor: depth.raised.shadowColor,
                shadowOpacity: depth.raised.shadowOpacity,
                shadowRadius: depth.raised.shadowRadius,
                shadowOffset: depth.raised.shadowOffset,
                elevation: depth.raised.elevation,
              }
            : null,
        ]}
      />
    </Screen>
  );
}

const FAB_SIZE = 56;

const styles = StyleSheet.create({
  list: { paddingTop: space.space12 },
  listContent: { paddingTop: space.space12, paddingBottom: 96 },
  header: { flexDirection: "row", marginBottom: space.space8 },
  headerStat: { flex: 1, gap: space.space4, alignItems: "flex-start" },
  headerDivider: { borderLeftWidth: StyleSheet.hairlineWidth, paddingLeft: space.space12 },
  skeletonCard: { marginBottom: space.space8, gap: space.space8 },
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
