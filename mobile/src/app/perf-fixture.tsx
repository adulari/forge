import React, { useEffect, useMemo, useRef, useState } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";

import { getDesktopPerformanceSnapshot, type DesktopPerformanceSnapshot } from "../lib/performance";
import { useTokens } from "../theme/ThemeProvider";
import { type } from "../theme/typography";

const ROW_COUNT = 10_000;
const STREAM_TOKEN_COUNT = 600;
const STREAM_INTERVAL_MS = 50;
const RELEASE_PERF_FIXTURE = process.env.EXPO_PUBLIC_PERF_FIXTURE === "1";
const PERF_FIXTURE_ENABLED = __DEV__ || RELEASE_PERF_FIXTURE;

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
  const listRef = useRef<FlatList<FixtureRow>>(null);

  useEffect(() => {
    if (typeof document !== "undefined") {
      document.title = `Perf fixture | startup=${snapshot.startupToInteractiveMs?.toFixed(1) ?? "pending"}ms | frames=${snapshot.frameSamples} | dropped=${snapshot.droppedFrames} | long=${snapshot.longestTaskMs.toFixed(1)}ms`;
    }
  }, [snapshot]);
  useEffect(() => {
    if (!PERF_FIXTURE_ENABLED) return;

    const refresh = setInterval(() => setSnapshot(getDesktopPerformanceSnapshot()), 1_000);
    let emitted = 0;
    let scrollOffset = 0;
    let direction = 1;
    const stream = setInterval(() => {
      emitted += 1;
      setStreamedTokens(emitted);
      if (emitted >= STREAM_TOKEN_COUNT) clearInterval(stream);
      scrollOffset += direction * 480;
      if (scrollOffset >= ROW_COUNT * 20) direction = -1;
      if (scrollOffset <= 0) direction = 1;
      listRef.current?.scrollToOffset({ offset: Math.max(0, scrollOffset), animated: false });
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

  if (!PERF_FIXTURE_ENABLED) {
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
        ref={listRef}
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
