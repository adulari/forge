// Hearth Session Replay: "a chronological record of this session" — de-boxed, hairline-
// separated entries (core rule 1), each headed by a mono uppercase role label (you=accent,
// forge=ink3, system=ink4 — the prototype's fourth "tool output" label isn't a real distinct
// HistoryRow.role, so a multi-line system row renders through the existing collapsible
// SystemOutput box while a single-line one stays plain text, matching the prototype's two
// system-row treatments off the actual content shape rather than a fabricated field).
import { ArrowLeft, Clock } from "lucide-react-native";
import { router, useLocalSearchParams } from "expo-router";
import React, { memo, useCallback, useMemo } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { Markdown } from "../../../components/chat/Markdown";
import { SystemOutput } from "../../../components/chat/SystemOutput";
import { BoundedList } from "../../../components/ds/BoundedList";
import { Button } from "../../../components/ds/Button";
import { EmptyState } from "../../../components/ds/EmptyState";
import { IconButton } from "../../../components/ds/IconButton";
import { Screen } from "../../../components/ds/Screen";
import { type HistoryRow } from "../../../lib/api";
import { useHistory } from "../../../lib/queries";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { tabularNums, type as typeScale } from "../../../theme/typography";

const roleLabel = (role: HistoryRow["role"]) =>
  role === "user" ? "you" : role === "assistant" ? "forge" : "system";

function roleColor(role: HistoryRow["role"], tokens: ReturnType<typeof useTokens>) {
  if (role === "user") return tokens.accent;
  if (role === "assistant") return tokens.ink3;
  return tokens.ink4;
}

function formatClock(epochSeconds: number) {
  return new Date(epochSeconds * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
}

export default function SessionReplayScreen() {
  const tokens = useTokens();
  const { id } = useLocalSearchParams<{ id: string }>();
  const query = useHistory(id ?? null);
  const rows = useMemo(() => query.data?.pages.flat().slice().reverse() ?? [], [query.data?.pages]);
  const renderItem = useCallback(({ item }: { item: HistoryRow; index: number }) => <ReplayRow row={item} />, []);

  return (
    <Screen contentContainerStyle={styles.sessionColumn}>
      <BoundedList
        data={rows}
        renderItem={renderItem}
        keyExtractor={(row) => String(row.seq)}
        refreshing={query.isFetching && !query.isFetchingNextPage}
        onRefresh={() => void query.refetch()}
        loadingMore={query.isFetchingNextPage}
        onEndReached={() => {
          if (query.hasNextPage && !query.isFetchingNextPage) void query.fetchNextPage();
        }}
        ListHeaderComponent={
          <View style={styles.header}>
            <View style={styles.headerRow}>
              <IconButton
                icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink2} />}
                onPress={() => router.back()}
                accessibilityLabel="Back to session"
              />
              <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Replay</Text>
            </View>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>A chronological record of this session.</Text>
          </View>
        }
        ListEmptyComponent={
          query.isLoading ? (
            <View style={styles.loading}>
              <ActivityIndicator color={tokens.accent} />
              <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Loading replay…</Text>
            </View>
          ) : query.isError ? (
            <EmptyState icon={Clock} message="Could not load this replay." action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />} />
          ) : (
            <EmptyState icon={Clock} message="No saved messages yet." />
          )
        }
        contentContainerStyle={styles.content}
      />
    </Screen>
  );
}

const ReplayRow = memo(function ReplayRow({ row }: { row: HistoryRow }) {
  const tokens = useTokens();
  const color = roleColor(row.role, tokens);
  const multiline = row.role === "system" && row.content.includes("\n");

  return (
    <View style={styles.entry}>
      <Text style={[typeScale.section, { color }]}>
        {roleLabel(row.role)} · <Text style={tabularNums}>{formatClock(row.created_at)}</Text>
      </Text>
      {multiline ? (
        <View style={styles.entryBody}>
          <SystemOutput content={row.content} />
        </View>
      ) : row.role === "system" ? (
        <Text style={[typeScale.sub, styles.entryBody, { color: tokens.ink3 }]}>{row.content}</Text>
      ) : (
        <View style={styles.entryBody}>
          <Markdown content={row.content} />
        </View>
      )}
      <View style={[styles.separator, { backgroundColor: tokens.hairline }]} />
    </View>
  );
});

const styles = StyleSheet.create({
  sessionColumn: { width: "100%", maxWidth: 760, alignSelf: "center" },
  content: { paddingHorizontal: space.space20, paddingTop: space.space8, paddingBottom: space.space32 },
  header: { gap: space.space4, marginBottom: space.space8 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginLeft: -space.space8 },
  loading: { alignItems: "center", padding: space.space32, gap: space.space12 },
  entry: { paddingTop: space.space16 },
  entryBody: { marginTop: space.space8 },
  separator: { height: StyleSheet.hairlineWidth, marginTop: space.space16 },
});
