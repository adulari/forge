import React, { useEffect, useMemo, useState } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";

import { getDesktopPerformanceSnapshot, type DesktopPerformanceSnapshot } from "../lib/performance";
import { useTokens } from "../theme/ThemeProvider";
import { type } from "../theme/typography";

const ROW_COUNT = 10_000;
const STREAM_TOKEN_COUNT = 600;
const STREAM_INTERVAL_MS = 50;

type FixtureRow = { id: string; index: number; text: string };

const rows: FixtureRow[] = Array.from({ length: ROW_COUNT }, (_, index) => ({
  id: `fixture-${index}`,
  index,
  text: `Deterministic transcript fixture row ${index + 1}: stable content for virtualization measurement.`,
}));

export default function PerformanceFixtureScreen() {
  const tokens = useTokens();
  const [snapshot, setSnapshot] = useState<DesktopPerformanceSnapshot>(() => getDesktopPerformanceSnapshot());
  const [streamedTokens, setStreamedTokens] = useState(0);

  useEffect(() => {
    if (!__DEV__) return;
    const refresh = setInterval(() => setSnapshot(getDesktopPerformanceSnapshot()), 1_000);
    let emitted = 0;
    const stream = setInterval(() => {
      emitted += 1;
      setStreamedTokens(emitted);
      if (emitted >= STREAM_TOKEN_COUNT) clearInterval(stream);
    }, STREAM_INTERVAL_MS);
    return () => {
      clearInterval(refresh);
      clearInterval(stream);
    };
  }, []);

  const header = useMemo(
    () => (
      <View style={styles.header}>
        <Text style={[type.title, { color: tokens.ink }]}>Desktop performance fixture</Text>
        <Text style={[type.sub, { color: tokens.ink2 }]}>10,000 deterministic rows · local mock stream: {streamedTokens}/{STREAM_TOKEN_COUNT} tokens at {STREAM_INTERVAL_MS} ms intervals</Text>
        <Text style={[type.codeSmall, { color: tokens.ink3 }]}>startup {snapshot.startupToInteractiveMs?.toFixed(1) ?? "pending"} ms · frames {snapshot.frameSamples} · dropped {snapshot.droppedFrames} · long tasks {snapshot.longTaskCount} / {snapshot.longestTaskMs.toFixed(1)} ms max</Text>
      </View>
    ),
    [snapshot, streamedTokens, tokens],
  );

  if (!__DEV__) {
    return (
      <View style={[styles.screen, { backgroundColor: tokens.bg1 }]}>
        <Text style={[type.title, { color: tokens.ink }]}>Performance fixture unavailable</Text>
      </View>
    );
  }

  return (
    <View style={[styles.screen, { backgroundColor: tokens.bg1 }]}>
      <FlatList
        data={rows}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => <Text style={[styles.row, { color: tokens.ink2 }]}>{item.text}</Text>}
        ListHeaderComponent={header}
        initialNumToRender={24}
        maxToRenderPerBatch={24}
        windowSize={9}
        removeClippedSubviews
      />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  header: { gap: 8, padding: 24 },
  row: { fontFamily: "monospace", fontSize: 12, lineHeight: 20, paddingHorizontal: 24 },
});
