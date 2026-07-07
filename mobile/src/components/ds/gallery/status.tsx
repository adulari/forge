// T1.2 gallery section — every Status & data component, every state/tone, both
// themes. Imported by T1.3's `src/app/gallery.tsx` registry.
import React, { useState } from "react";
import { ScrollView, StyleSheet, View } from "react-native";

import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { Badge } from "../Badge";
import { ContextGauge } from "../ContextGauge";
import { CostMetric } from "../CostMetric";
import { KeyValueRow } from "../KeyValueRow";
import { RelativeTime } from "../RelativeTime";
import { SectionHeader } from "../SectionHeader";
import { StatusDot } from "../StatusDot";

export default function StatusGallery() {
  const tokens = useTokens();
  const [now] = useState(() => Date.now());

  return (
    <ScrollView
      style={{ backgroundColor: tokens.bg1 }}
      contentContainerStyle={styles.content}
    >
      <SectionHeader>Status dot</SectionHeader>
      <View style={styles.row}>
        <StatusDot state="idle" />
        <StatusDot state="busy" />
        <StatusDot state="waiting" />
        <StatusDot state="done" />
      </View>

      <SectionHeader>Badge — small</SectionHeader>
      <View style={styles.row}>
        <Badge label="worktree" tone="neutral" />
        <Badge label="archived" tone="neutral" />
        <Badge label="public" tone="danger" />
        <Badge label="beta" tone="accent" />
        <Badge label="ok" tone="success" />
        <Badge label="caution" tone="warn" />
        <Badge label="outline" tone="outline" />
      </View>

      <SectionHeader>Badge — pill</SectionHeader>
      <View style={styles.row}>
        <Badge label="NEEDS YOU" tone="danger" shape="pill" />
        <Badge label="worktree" tone="neutral" shape="pill" />
        <Badge label="live" tone="accent" shape="pill" />
      </View>

      <SectionHeader>Context gauge</SectionHeader>
      <View style={styles.stack}>
        <ContextGauge used={45_000} total={200_000} />
        <ContextGauge used={148_000} total={200_000} />
        <ContextGauge used={192_000} total={200_000} />
      </View>

      <SectionHeader>Cost metric</SectionHeader>
      <View style={styles.row}>
        <CostMetric valueUsd={0.0421} />
        <CostMetric valueUsd={12.48} />
      </View>

      <SectionHeader>Key value row</SectionHeader>
      <View>
        <KeyValueRow label="Server" value="forge.local" />
        <KeyValueRow label="Appearance" value="System" chevron onPress={() => {}} />
        <KeyValueRow label="App lock" chevron onPress={() => {}} />
      </View>

      <SectionHeader>Relative time</SectionHeader>
      <View style={styles.row}>
        <RelativeTime timestampMs={now - 12_000} />
        <RelativeTime timestampMs={now - 4 * 60_000} />
        <RelativeTime timestampMs={now - 2 * 60 * 60_000} />
        <RelativeTime timestampMs={now - 3 * 24 * 60 * 60_000} />
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    paddingBottom: space.space48,
  },
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: space.space12,
    paddingHorizontal: space.space16,
  },
  stack: {
    gap: space.space12,
    paddingHorizontal: space.space16,
  },
});
