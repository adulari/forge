// Fleet — the app home + most-used surface (BUILD_PLAN §6 "Fleet"). Server-sorted
// (waiting first), polled every 3s while focused (useSessions), warm-started from the
// persisted query cache so cold open never shows a spinner over stale data.
import { router } from "expo-router";
import React, { useCallback, useMemo } from "react";
import { Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { ApiError, type SessionRow } from "../../lib/api";
import { usePulse } from "../../lib/motion";
import { useSessions } from "../../lib/queries";
import {
  Badge,
  BoundedList,
  Card,
  EmptyState,
  ErrorText,
  FAB,
  ListRow,
  Metric,
  Screen,
  StatusDot,
  type StatusDotState,
  type Tone,
  EntranceView,
} from "../../components/ui";
import { type SessionActionTarget, useSessionActions } from "../../components/sessionActions";

function formatTokenCount(n: number): string {
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function contextTone(pct: number): Tone {
  if (pct > 0.9) return "no";
  if (pct > 0.7) return "accent";
  return "dim";
}

const contextBarClass: Record<Tone, string> = {
  no: "bg-no",
  accent: "bg-accent",
  dim: "bg-dim",
  ok: "bg-ok",
  ink: "bg-ink",
};

const contextTextClass: Record<Tone, string> = {
  no: "text-no",
  accent: "text-accent",
  dim: "text-dim",
  ok: "text-ok",
  ink: "text-ink",
};

function ContextGauge({ tokens, limit }: { tokens: number; limit: number | null }) {
  if (!limit) {
    return (
      <Text className="text-dim text-[12px]" style={{ fontVariant: ["tabular-nums"] }}>
        {formatTokenCount(tokens)}
      </Text>
    );
  }
  const pct = Math.min(1, tokens / limit);
  const tone = contextTone(pct);
  return (
    <View className="items-end gap-2">
      <Text
        className={`text-[12px] ${contextTextClass[tone]}`}
        style={{ fontVariant: ["tabular-nums"] }}
      >
        {formatTokenCount(tokens)}/{formatTokenCount(limit)}
      </Text>
      <View className="w-[40px] h-[3px] rounded-full bg-borderSoft overflow-hidden">
        <View
          className={`h-[3px] rounded-full ${contextBarClass[tone]}`}
          style={{ width: `${pct * 100}%` }}
        />
      </View>
    </View>
  );
}

interface FleetRowProps {
  row: SessionRow;
  index: number;
  onOpenActions: (target: SessionActionTarget) => void;
}

function FleetRowBase({ row, index, onOpenActions }: FleetRowProps) {
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const title = row.title || `#${row.id.slice(0, 8)}`;

  return (
    <EntranceView index={index}>
      <ListRow
        title={title}
        subtitle={row.cwd}
        subtitleEllipsize="head"
        left={<StatusDot state={state} />}
        onPress={() => router.push(`/session/${row.id}`)}
        onLongPress={() => onOpenActions({ id: row.id, title, worktree: row.worktree })}
        right={
          <View className="items-end gap-4" style={{ minWidth: 96 }}>
            {row.waiting ? <Badge label="NEEDS YOU" tone="no" /> : null}
            <View className="flex-row items-center gap-4">
              {row.worktree ? <Badge label="⎇" tone="default" /> : null}
              <Badge label={row.model} tone="default" />
            </View>
            <View className="flex-row items-center gap-8">
              <Metric value={row.cost_usd} format="cost" tone="ok" />
              <ContextGauge tokens={row.context_tokens} limit={row.context_limit} />
            </View>
          </View>
        }
      />
    </EntranceView>
  );
}

const FleetRow = React.memo(FleetRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.onOpenActions === next.onOpenActions &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.worktree === b.worktree &&
    a.busy === b.busy &&
    a.waiting === b.waiting &&
    a.cost_usd === b.cost_usd &&
    a.context_tokens === b.context_tokens &&
    a.context_limit === b.context_limit &&
    a.model === b.model
  );
});

function FleetRowSkeleton() {
  const pulseStyle = usePulse("busy");
  return (
    <Animated.View style={pulseStyle}>
      <Card className="gap-6 mb-8">
        <View className="flex-row items-center gap-8">
          <View className="w-8 h-8 rounded-full bg-borderSoft" />
          <View className="flex-1 gap-6">
            <View className="h-12 rounded-sm bg-borderSoft" style={{ width: "55%" }} />
            <View className="h-10 rounded-sm bg-borderSoft" style={{ width: "35%" }} />
          </View>
          <View className="w-[40px] h-10 rounded-sm bg-borderSoft" />
        </View>
      </Card>
    </Animated.View>
  );
}

export default function FleetScreen() {
  const query = useSessions();
  const { open, sheet } = useSessionActions();

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => (
      <FleetRow row={item} index={index} onOpenActions={open} />
    ),
    [open],
  );
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);
  const emptyComponent = useMemo(
    () => (
      <EmptyState
        title="No live sessions — start one"
        action={{ label: "New session", onPress: () => router.push("/new-session") }}
      />
    ),
    [],
  );

  const data = query.data ?? [];
  const hasData = data.length > 0;

  if (query.isLoading) {
    return (
      <Screen>
        <View className="pt-8">
          {[0, 1, 2, 3, 4].map((i) => (
            <FleetRowSkeleton key={i} />
          ))}
        </View>
      </Screen>
    );
  }

  if (query.isError && !hasData) {
    const message = query.error instanceof ApiError ? query.error.message : "server unreachable";
    return (
      <Screen>
        <ErrorText message={message} onRetry={() => query.refetch()} />
      </Screen>
    );
  }

  return (
    <Screen scroll={false}>
      {query.isError && hasData ? (
        <Text className="text-no text-[12px] pt-6 pb-2">
          Couldn't refresh — showing last known state.
        </Text>
      ) : null}
      <BoundedList
        data={data}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
        refreshing={query.isRefetching}
        onRefresh={query.refetch}
      />
      <FAB label="New" onPress={() => router.push("/new-session")} />
      {sheet}
    </Screen>
  );
}
