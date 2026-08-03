import { getCurrentWindow } from "@tauri-apps/api/window";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { FlatList, StyleSheet, Text, TextInput, View } from "react-native";

import { dumpDesktopPerformanceSnapshot, getDesktopPerformanceSnapshot, markFirstWorkloadEvent, markPerformancePhaseStart, recordCompositorKey, recordComposerInput, type DesktopPerformanceSnapshot } from "../lib/performance";
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
  const [draft, setDraft] = useState("");
  const [phase, setPhase] = useState("idle");
  const listRef = useRef<FlatList<FixtureRow>>(null);

  useEffect(() => {
    if (!PERF_FIXTURE_ENABLED) return;
    void (async () => {
    const { invoke } = await import("@tauri-apps/api/core");
      const selectedPhase = await invoke<string>("perf_phase");
      setPhase(selectedPhase);
      markPerformancePhaseStart();
    })();
  }, []);

  useEffect(() => {
    if (!PERF_FIXTURE_ENABLED) return;
    window.addEventListener("keydown", recordCompositorKey);
    return () => window.removeEventListener("keydown", recordCompositorKey);
  }, []);

  useEffect(() => {
    const title = `Perf fixture | phase=${phase} | startup=${snapshot.startupToInteractiveMs?.toFixed(1) ?? "pending"}ms | frames=${snapshot.frameSamples} | dropped=${snapshot.droppedFrames} | long=${snapshot.longestTaskMs.toFixed(1)}ms`;
    void getCurrentWindow().setTitle(title);
    if (typeof document !== "undefined") document.title = title;
  }, [phase, snapshot]);
  useEffect(() => {
    if (!PERF_FIXTURE_ENABLED) return;

    const refresh = setInterval(() => {
      setSnapshot(getDesktopPerformanceSnapshot());
      void dumpDesktopPerformanceSnapshot();
    }, 1_000);
    let emitted = 0;
    let scrollOffset = 0;
    let direction = 1;
    const driveScroll = () => {
      markFirstWorkloadEvent();
      scrollOffset += direction * 480;
      if (scrollOffset >= ROW_COUNT * 20) direction = -1;
      if (scrollOffset <= 0) direction = 1;
      listRef.current?.scrollToOffset({ offset: Math.max(0, scrollOffset), animated: false });
    };
    const scroll = phase === "scroll" ? setInterval(driveScroll, STREAM_INTERVAL_MS) : null;
    const stream = phase === "stream" ? setInterval(() => {
      emitted += 1;
      setStreamedTokens(emitted);
      driveScroll();
    }, STREAM_INTERVAL_MS) : null;
    return () => {
      void dumpDesktopPerformanceSnapshot();
      clearInterval(refresh);
      if (scroll != null) clearInterval(scroll);
      if (stream != null) clearInterval(stream);
    };
  }, [phase]);

  const header = useMemo(
    () => (
      <View style={styles.header}>
        <Text style={[type.title, { color: tokens.ink }]}>Desktop performance fixture</Text>
        <Text style={[type.sub, { color: tokens.ink2 }]}>10,000 deterministic rows · local mock stream: {streamedTokens}/{STREAM_TOKEN_COUNT} tokens at {STREAM_INTERVAL_MS} ms intervals</Text>
        <TextInput
          autoFocus
          value={draft}
          onChangeText={(next) => {
            recordComposerInput();
            setDraft(next);
          }}
          placeholder="Typing capture input"
          style={[styles.input, { color: tokens.ink, borderColor: tokens.ink3 }]}
        />
      </View>
    ),
    [draft, streamedTokens, tokens],
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
  input: { borderWidth: 1, borderRadius: 6, padding: 8, minHeight: 40 },
  row: { fontFamily: "monospace", fontSize: 12, lineHeight: 20, paddingHorizontal: 24 },
});
