import { Clock } from "lucide-react-native";
import { router, useLocalSearchParams } from "expo-router";
import React, { memo, useCallback, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Markdown } from "../../../components/chat/Markdown";
import { BoundedList } from "../../../components/ds/BoundedList";
import { Card } from "../../../components/ds/Card";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { type HistoryRow } from "../../../lib/api";
import { useHistory } from "../../../lib/queries";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type } from "../../../theme/typography";

export default function SessionReplayScreen() {
  const tokens = useTokens();
  const { id } = useLocalSearchParams<{ id: string }>();
  const query = useHistory(id ?? null);
  const rows = useMemo(() => query.data?.pages.flat().slice().reverse() ?? [], [query.data?.pages]);
  const renderItem = useCallback(({ item }: { item: HistoryRow; index: number }) => <ReplayRow row={item} />, []);

  return (
    <Screen contentContainerStyle={styles.screen}>
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
            <Pressable onPress={() => router.back()} accessibilityRole="button" accessibilityLabel="Back to session" style={styles.backButton}>
              <Text style={[styles.back, { color: tokens.accent }]}>‹ Session</Text>
            </Pressable>
            <Text style={[type.title, { color: tokens.ink }]}>Session replay</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>A chronological record of this session.</Text>
          </View>
        }
        ListEmptyComponent={
          query.isLoading ? (
            <View />
          ) : (
            <EmptyState icon={Clock} message={query.isError ? "Could not load this replay. Pull to retry." : "No saved messages yet."} />
          )
        }
        contentContainerStyle={styles.content}
      />
    </Screen>
  );
}

const ReplayRow = memo(function ReplayRow({ row }: { row: HistoryRow }) {
  const tokens = useTokens();
  return (
    <Card style={styles.card}>
      <Text style={[type.sub, { color: row.role === "user" ? tokens.accent : tokens.ink3 }]}>
        {row.role === "assistant" ? row.model ?? "Forge" : row.role}
      </Text>
      <Markdown content={row.content} />
    </Card>
  );
});

const styles = StyleSheet.create({
  screen: { paddingHorizontal: 0 },
  content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  header: { gap: space.space12, marginBottom: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  backButton: { minHeight: 44, alignSelf: "flex-start", justifyContent: "center", paddingRight: space.space12 },
  card: { gap: space.space8 },
});
