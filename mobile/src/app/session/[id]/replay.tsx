import { Clock } from "lucide-react-native";
import { router, useLocalSearchParams } from "expo-router";
import React, { useMemo } from "react";
import { Pressable, RefreshControl, StyleSheet, Text } from "react-native";

import { Card } from "../../../components/ds/Card";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { Markdown } from "../../../components/chat/Markdown";
import { useHistory } from "../../../lib/queries";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type } from "../../../theme/typography";

export default function SessionReplayScreen() {
  const tokens = useTokens();
  const { id } = useLocalSearchParams<{ id: string }>();
  const query = useHistory(id ?? null);
  const rows = useMemo(() => query.data?.pages.flat().slice().reverse() ?? [], [query.data?.pages]);
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Session</Text></Pressable><Text style={[type.title, { color: tokens.ink }]}>Session replay</Text><Text style={[type.sub, { color: tokens.ink3 }]}>A chronological record of this session.</Text>{rows.length === 0 && !query.isLoading ? <EmptyState icon={Clock} message="No saved messages yet." /> : null}{rows.map((row) => <Card key={row.seq} style={styles.card}><Text style={[type.sub, { color: row.role === "user" ? tokens.accent : tokens.ink3 }]}>{row.role === "assistant" ? row.model ?? "Forge" : row.role}</Text><Markdown content={row.content} /></Card>)}</Screen>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, card: { gap: space.space8 } });
