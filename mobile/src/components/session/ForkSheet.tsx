import { router } from "expo-router";
import React, { useMemo } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { HistoryRow } from "../../lib/api";
import { useForkSession, useHistory } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Sheet } from "../ds/Sheet";

function preview(row: HistoryRow) {
  return row.content.replace(/\s+/g, " ").trim();
}

export function ForkSheet({ visible, onClose, sessionId }: { visible: boolean; onClose: () => void; sessionId: string }) {
  const tokens = useTokens();
  const history = useHistory(sessionId);
  const fork = useForkSession();
  const prompts = useMemo(() => history.data?.pages.flat().filter((row) => row.role === "user").slice().reverse() ?? [], [history.data?.pages]);
  const startFork = (row: HistoryRow) => fork.mutate({ id: sessionId, body: { at_seq: row.seq } }, { onSuccess: (session) => { onClose(); router.push(`/session/${session.id}`); } });

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Fork session" snapPoints={[0.75]}>
      <View style={styles.header}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Fork session</Text>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Branch before a prompt and explore a different path. Files are unchanged.</Text>
      </View>
      <ScrollView contentContainerStyle={styles.options} keyboardShouldPersistTaps="handled">
        {prompts.map((row) => <Pressable key={row.seq} onPress={() => startFork(row)} disabled={fork.isPending} accessibilityRole="button" style={[styles.option, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}><Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={2}>{preview(row)}</Text><Text style={[typeScale.meta, { color: tokens.accent }]}>Fork before this prompt</Text></Pressable>)}
        {prompts.length === 0 ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No user prompts to fork from yet.</Text> : null}
        {fork.isError ? <Text style={[typeScale.sub, { color: tokens.danger }]} numberOfLines={2}>Could not fork this session. Try again.</Text> : null}
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({ header: { paddingHorizontal: space.space16, gap: space.space12 }, options: { padding: space.space16, gap: space.space12 }, option: { minHeight: 44, justifyContent: "center", padding: space.space12, gap: 2, borderWidth: 1, borderRadius: 8 } });
