// Search a session's whole history, including the messages a compaction folded out of the model's
// context. Tapping a hit closes the sheet and jumps the transcript to that message.
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { requestMessageJump } from "../../lib/messageJump";
import { useHistorySearch } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { SearchField } from "../ds/SearchField";
import { Sheet } from "../ds/Sheet";
import { emptyMessage, roleLabel, splitSnippet, visibleHits } from "./historySearchModel";

export function HistorySearchSheet({
  visible,
  onClose,
  sessionId,
}: {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
}) {
  const tokens = useTokens();
  const [query, setQuery] = useState("");
  const search = useHistorySearch(sessionId, query, visible);
  const hits = visibleHits(search.data?.hits ?? []);

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Search history" snapPoints={[0.8]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Search history</Text>
        <SearchField
          value={query}
          onChangeText={setQuery}
          placeholder="Words from the conversation"
          accessibilityLabel="Search this session's history"
          showCancel={false}
          autoFocus
        />
        {search.error ? (
          <Text style={[typeScale.sub, { color: tokens.danger }]}>{search.error.message}</Text>
        ) : hits.length === 0 ? (
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            {search.isFetching ? "Searching…" : emptyMessage(query)}
          </Text>
        ) : (
          hits.map((hit) => (
            <Pressable
              key={hit.seq}
              accessibilityRole="button"
              accessibilityLabel={`${roleLabel(hit.role)}, message ${hit.seq}`}
              onPress={() => {
                requestMessageJump(sessionId, hit.seq);
                onClose();
              }}
              style={[styles.hit, { borderColor: tokens.hairline }]}
            >
              <View style={styles.meta}>
                <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
                  {roleLabel(hit.role)} · #{hit.seq}
                </Text>
                {hit.folded ? <Badge label="folded" tone="outline" /> : null}
              </View>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
                {splitSnippet(hit.snippet).map((part, index) => (
                  <Text key={index} style={part.match ? { color: tokens.ink, fontWeight: "600" } : undefined}>
                    {part.text}
                  </Text>
                ))}
              </Text>
            </Pressable>
          ))
        )}
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space16, gap: space.space12 },
  hit: { paddingVertical: space.space8, gap: space.space4, borderBottomWidth: StyleSheet.hairlineWidth },
  meta: { flexDirection: "row", alignItems: "center", gap: space.space4 },
});
