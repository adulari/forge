import { router } from "expo-router";
import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useConfig } from "../../lib/queries";
import { configHref, searchSettings, type SettingsHit } from "../../lib/settingsSearch";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { SearchField } from "../ds/SearchField";

export function openSettingsHit(hit: SettingsHit) {
  router.push(hit.kind === "field" ? configHref(hit.focusKey) : hit.href);
}

export function useSettingsSearch() {
  const [query, setQuery] = useState("");
  const config = useConfig();
  const hits = useMemo(() => searchSettings(query, config.data?.fields ?? []), [query, config.data?.fields]);
  return { query, setQuery, hits };
}

export function SettingsSearchResults({ query, hits, onPick }: { query: string; hits: SettingsHit[]; onPick?: () => void }) {
  const tokens = useTokens();
  if (!query.trim()) return null;
  if (hits.length === 0) return <Text style={[type.sub, styles.empty, { color: tokens.ink3 }]}>No settings match that search.</Text>;
  return (
    <View accessibilityRole="list" style={styles.list}>
      {hits.map((hit) => (
        <Pressable
          key={hit.id}
          accessibilityRole="button"
          accessibilityLabel={`${hit.title}, ${hit.subtitle}`}
          onPress={() => {
            openSettingsHit(hit);
            onPick?.();
          }}
          style={({ pressed }) => [styles.row, { backgroundColor: pressed ? tokens.selection : "transparent", borderRadius: radii.radius4 }]}
        >
          <Text style={[type.bodyBold, { color: tokens.ink }]} numberOfLines={1}>{hit.title}</Text>
          <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={1}>{hit.subtitle}</Text>
        </Pressable>
      ))}
    </View>
  );
}

export function SettingsSearchField({ query, setQuery, hits, onPick }: { query: string; setQuery: (value: string) => void; hits: SettingsHit[]; onPick?: () => void }) {
  return (
    <SearchField
      value={query}
      onChangeText={setQuery}
      placeholder="Search settings"
      accessibilityLabel="Search all settings"
      returnKeyType="go"
      onSubmitEditing={() => {
        if (hits[0]) {
          openSettingsHit(hits[0]);
          onPick?.();
        }
      }}
    />
  );
}

const styles = StyleSheet.create({
  list: { gap: space.space4 },
  row: { paddingHorizontal: space.space8, paddingVertical: space.space8, gap: 2 },
  empty: { paddingHorizontal: space.space8, paddingVertical: space.space8 },
});
