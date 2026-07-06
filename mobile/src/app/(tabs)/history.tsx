// History — past-session browser + resurrection (BUILD_PLAN §6 "History", Batch 1 W3).
// Infinite/cursor scroll over usePastSessions() (`before` = last row's last_activity),
// client-side search filter over title/cwd/preview, tap-to-resume via useCreateSession.
import { router } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { Alert, Text, View } from "react-native";

import type { PastSessionRow } from "../../lib/api";
import { ApiError } from "../../lib/api";
import {
  Badge,
  BoundedList,
  Card,
  EmptyState,
  EntranceView,
  ErrorText,
  formatMetric,
  Loading,
  Metric,
  Screen,
  SearchInput,
  SectionTitle,
} from "../../components/ui";
import { useCreateSession, usePastSessions } from "../../lib/queries";
import { theme } from "../../lib/theme";

function formatRelativeTime(unixSeconds: number): string {
  const diffSec = Math.max(0, Date.now() / 1000 - unixSeconds);
  if (diffSec < 60) return "just now";
  const mins = Math.floor(diffSec / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(days / 365)}y ago`;
}

function matchesQuery(row: PastSessionRow, query: string): boolean {
  if (!query) return true;
  const haystack = `${row.title} ${row.cwd} ${row.preview ?? ""}`.toLowerCase();
  return haystack.includes(query);
}

interface HistoryRowProps {
  row: PastSessionRow;
  index: number;
  onPress: (row: PastSessionRow) => void;
}

function HistoryRowBase({ row, index, onPress }: HistoryRowProps) {
  return (
    <EntranceView index={index} style={{ marginBottom: 8 }}>
      <Card onPress={() => onPress(row)} className="gap-4">
        <View className="flex-row items-center justify-between gap-8">
          <Text numberOfLines={1} className="flex-1 text-ink text-[15px] font-semibold">
            {row.title || row.id.slice(0, 8)}
          </Text>
          {row.archived ? <Badge label="ARCHIVED" tone="accent" /> : null}
        </View>
        <Text
          numberOfLines={1}
          ellipsizeMode="head"
          className="text-dim text-[12px]"
        >
          {row.cwd}
        </Text>
        {row.preview ? (
          <Text numberOfLines={2} className="text-dim text-[13px]">
            {row.preview}
          </Text>
        ) : null}
        <View className="flex-row items-center justify-between gap-8 mt-2">
          <Text className="text-dim text-[12px]">{formatRelativeTime(row.last_activity)}</Text>
          <View className="flex-row items-center gap-10">
            <Text className="text-dim text-[12px]" style={{ fontVariant: ["tabular-nums"] }}>
              {formatMetric(row.message_count, "int")} msgs
            </Text>
            <Metric value={row.cost_usd} format="cost" tone="ok" />
          </View>
        </View>
      </Card>
    </EntranceView>
  );
}

const HistoryRow = React.memo(HistoryRowBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.archived === b.archived &&
    a.message_count === b.message_count &&
    a.cost_usd === b.cost_usd &&
    a.preview === b.preview &&
    a.last_activity === b.last_activity &&
    prev.index === next.index
  );
});

export default function HistoryScreen() {
  const [query, setQuery] = useState("");
  const {
    data,
    isLoading,
    isError,
    error,
    refetch,
    isRefetching,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePastSessions();
  const createSession = useCreateSession();
  const [resumingId, setResumingId] = useState<string | null>(null);

  const rows = useMemo(() => data?.pages.flat() ?? [], [data]);
  const normalizedQuery = query.trim().toLowerCase();
  const filteredRows = useMemo(
    () => rows.filter((row) => matchesQuery(row, normalizedQuery)),
    [rows, normalizedQuery],
  );

  const resume = useCallback(
    (row: PastSessionRow) => {
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
            const message = err instanceof ApiError ? err.message : "Could not resume session.";
            Alert.alert("Resume failed", message);
          },
        },
      );
    },
    [createSession],
  );

  const onRowPress = useCallback(
    (row: PastSessionRow) => {
      Alert.alert("Resume this session?", row.title || row.id.slice(0, 8), [
        { text: "Cancel", style: "cancel" },
        { text: "Resume", onPress: () => resume(row) },
      ]);
    },
    [resume],
  );

  const renderItem = useCallback(
    ({ item, index }: { item: PastSessionRow; index: number }) => (
      <HistoryRow row={item} index={index} onPress={onRowPress} />
    ),
    [onRowPress],
  );

  const keyExtractor = useCallback((item: PastSessionRow) => item.id, []);

  const onEndReached = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage) fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  let body: React.ReactNode;
  if (isLoading) {
    body = <Loading label="Loading past sessions…" />;
  } else if (isError) {
    body = (
      <ErrorText
        message={error instanceof ApiError ? error.message : "Could not load history."}
        onRetry={() => refetch()}
      />
    );
  } else {
    body = (
      <BoundedList
        data={filteredRows}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={
          <EmptyState
            glyph="◷"
            title={
              normalizedQuery
                ? "No past sessions match your search."
                : "No past sessions yet."
            }
          />
        }
        refreshing={isRefetching}
        onRefresh={refetch}
        onEndReached={onEndReached}
        onEndReachedThreshold={0.4}
        ListFooterComponent={isFetchingNextPage ? <Loading /> : null}
      />
    );
  }

  return (
    <Screen scroll={false}>
      <SectionTitle>History</SectionTitle>
      <SearchInput
        value={query}
        onChangeText={setQuery}
        placeholder="Search title or cwd…"
        autoCapitalize="none"
        autoCorrect={false}
        className="mb-8"
      />
      <View className="flex-1">{body}</View>
      {resumingId ? (
        <View
          className="absolute inset-0 items-center justify-center"
          style={{ backgroundColor: theme.colors.panelDeep }}
        >
          <Loading label="Resuming…" />
        </View>
      ) : null}
    </Screen>
  );
}
