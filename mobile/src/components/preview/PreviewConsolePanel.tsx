import { Trash2 } from "lucide-react-native";
import React, { useEffect, useRef } from "react";
import { FlatList, StyleSheet, Text, View, type ListRenderItemInfo } from "react-native";

import { clearConsole, isConsoleProblem, useConsoleEntries, type PreviewConsoleEntry } from "../../lib/previewConsole";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";

export const CONSOLE_PANEL_HEIGHT = 168;

export function PreviewConsolePanel({ label }: { label: string }) {
  const tokens = useTokens();
  const entries = useConsoleEntries(label);
  const listRef = useRef<FlatList<PreviewConsoleEntry>>(null);

  useEffect(() => {
    if (entries.length > 0) listRef.current?.scrollToEnd({ animated: false });
  }, [entries.length]);

  const colorFor = (level: PreviewConsoleEntry["level"]) =>
    level === "error" ? tokens.danger : level === "warn" ? tokens.warn : level === "debug" ? tokens.ink4 : tokens.ink2;

  const renderItem = ({ item }: ListRenderItemInfo<PreviewConsoleEntry>) => (
    <View
      style={[
        styles.line,
        { borderBottomColor: tokens.border },
        isConsoleProblem(item.level) && { backgroundColor: item.level === "error" ? tokens.dangerBg : tokens.warnBg },
      ]}
    >
      <Text style={[typeScale.monoMeta, styles.level, { color: colorFor(item.level) }]}>{item.level}</Text>
      <Text selectable style={[typeScale.monoMeta, styles.text, { color: colorFor(item.level) }]}>
        {item.text}
      </Text>
    </View>
  );

  return (
    <View
      style={[styles.root, { borderTopColor: tokens.border, backgroundColor: tokens.bg1 }]}
      accessibilityLabel="Preview console"
    >
      <View style={styles.header}>
        <Text style={[typeScale.meta, { color: tokens.ink3 }]}>
          Console · {entries.length} {entries.length === 1 ? "line" : "lines"}
        </Text>
        <IconButton
          icon={<Trash2 size={13} color={tokens.ink3} />}
          onPress={() => clearConsole(label)}
          disabled={entries.length === 0}
          accessibilityLabel="Clear console"
          style={styles.clear}
        />
      </View>
      {entries.length === 0 ? (
        <Text style={[typeScale.sub, styles.empty, { color: tokens.ink4 }]}>
          Nothing logged yet. Console output and uncaught errors from the page appear here.
        </Text>
      ) : (
        <FlatList ref={listRef} data={entries as PreviewConsoleEntry[]} renderItem={renderItem} keyExtractor={(item) => String(item.id)} />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { height: CONSOLE_PANEL_HEIGHT, flexShrink: 0, borderTopWidth: StyleSheet.hairlineWidth },
  header: {
    height: 28,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingLeft: space.space12,
    paddingRight: space.space4,
  },
  clear: { width: 26, height: 26 },
  empty: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
  line: {
    flexDirection: "row",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: 3,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  level: { width: 40 },
  text: { flex: 1 },
});
