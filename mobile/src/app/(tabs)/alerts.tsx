// Alerts — the fleet's killer signal: sessions waiting on YOU (BUILD_PLAN §6 "Alerts",
// Batch 1 W3). Derived from useSessions().filter(waiting). Calm when empty, prominent and
// pulsing when populated. Tap → session Chat segment, where the real permission/question
// card renders (SessionRow from /api/sessions carries only the `waiting` boolean — the
// actual prompt/question text lives in the per-session WS snapshot, so it can't be shown
// here without opening a socket per row; that's the Chat segment's job).
import { router } from "expo-router";
import React, { useCallback, useMemo } from "react";
import { Text, View } from "react-native";

import type { SessionRow } from "../../lib/api";
import { ApiError } from "../../lib/api";
import {
  Badge,
  BoundedList,
  Card,
  EmptyState,
  EntranceView,
  ErrorText,
  formatTokenCount,
  Loading,
  Metric,
  Screen,
  SectionTitle,
  StatusDot,
} from "../../components/ui";
import { useSessions } from "../../lib/queries";

interface AlertRowProps {
  row: SessionRow;
  index: number;
  onPress: (row: SessionRow) => void;
}

function AlertRowBase({ row, index, onPress }: AlertRowProps) {
  const ratio =
    row.context_limit && row.context_limit > 0
      ? row.context_tokens / row.context_limit
      : null;
  const gaugeTone = ratio == null ? "dim" : ratio > 0.9 ? "no" : ratio > 0.7 ? "accent" : "dim";
  const gaugeBarColor =
    gaugeTone === "no" ? "bg-no" : gaugeTone === "accent" ? "bg-accent" : "bg-dim";

  return (
    <EntranceView index={index} style={{ marginBottom: 8 }}>
      <Card onPress={() => onPress(row)} className="gap-6 border-no">
        <View className="flex-row items-center gap-8">
          <StatusDot state="waiting" />
          <Text numberOfLines={1} className="flex-1 text-ink text-[15px] font-semibold">
            {row.title || row.id.slice(0, 8)}
          </Text>
          <Badge label="NEEDS YOU" tone="no" />
        </View>
        <Text numberOfLines={1} className="text-no text-[13px] font-semibold">
          Waiting for your decision
        </Text>
        <Text numberOfLines={1} ellipsizeMode="head" className="text-dim text-[12px]">
          {row.cwd}
        </Text>
        <View className="flex-row items-center justify-between gap-8">
          <View className="flex-row items-center gap-6 flex-1">
            <Badge label={row.model} tone="default" />
            {row.worktree ? <Badge label="worktree" tone="default" /> : null}
          </View>
          <Metric value={row.cost_usd} format="cost" tone="ok" />
        </View>
        {row.context_limit ? (
          <View className="gap-4">
            <Text className="text-dim text-[12px]" style={{ fontVariant: ["tabular-nums"] }}>
              {formatTokenCount(row.context_tokens)}/{formatTokenCount(row.context_limit)}
            </Text>
            <View className="h-2 rounded-pill bg-border overflow-hidden">
              <View
                className={`h-2 rounded-pill ${gaugeBarColor}`}
                style={{ width: `${Math.min(100, Math.max(2, (ratio ?? 0) * 100))}%` }}
              />
            </View>
          </View>
        ) : null}
      </Card>
    </EntranceView>
  );
}

const AlertRow = React.memo(AlertRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.model === b.model &&
    a.worktree === b.worktree &&
    a.cost_usd === b.cost_usd &&
    a.context_tokens === b.context_tokens &&
    a.context_limit === b.context_limit &&
    prev.index === next.index
  );
});

export default function AlertsScreen() {
  const { data, isLoading, isError, error, refetch, isRefetching } = useSessions();

  const waiting = useMemo(() => (data ?? []).filter((s) => s.waiting), [data]);

  const onRowPress = useCallback((row: SessionRow) => {
    router.push(`/session/${row.id}`);
  }, []);

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => (
      <AlertRow row={item} index={index} onPress={onRowPress} />
    ),
    [onRowPress],
  );

  const keyExtractor = useCallback((item: SessionRow) => item.id, []);

  let body: React.ReactNode;
  if (isLoading) {
    body = <Loading label="Checking for sessions waiting on you…" />;
  } else if (isError) {
    body = (
      <ErrorText
        message={error instanceof ApiError ? error.message : "Could not load sessions."}
        onRetry={() => refetch()}
      />
    );
  } else {
    body = (
      <BoundedList
        data={waiting}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={
          <EmptyState glyph="✓" title="All clear — nothing needs you right now." />
        }
        refreshing={isRefetching}
        onRefresh={refetch}
      />
    );
  }

  return (
    <Screen scroll={false}>
      <SectionTitle>Alerts</SectionTitle>
      <View className="flex-1">{body}</View>
    </Screen>
  );
}
