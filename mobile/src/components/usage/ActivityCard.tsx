import { router } from "expo-router";
import React, { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Svg, { Rect } from "react-native-svg";

import { buildHeatmap, compactTokens, summarize } from "../../lib/usageActivity";
import { useUsageActivity } from "../../lib/useUsageActivity";
import { useToday } from "../../lib/useToday";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";

const WEEKS = 18;
const CELL = 8;
const GAP = 2;

export function ActivityCard() {
  const tokens = useTokens();
  const { data } = useUsageActivity();
  const today = useToday(data);
  const rows = data?.days;
  const weeks = useMemo(() => buildHeatmap(rows ?? [], today, "all").slice(-WEEKS), [rows, today]);
  const summary = useMemo(() => summarize(rows ?? [], today, "30"), [rows, today]);
  if (!rows || rows.length === 0) return null;
  const levels = [tokens.border, `${tokens.accent}55`, `${tokens.accent}88`, `${tokens.accent}BB`, tokens.accent];
  return (
    <Pressable
      onPress={() => router.push("/usage")}
      accessibilityRole="button"
      accessibilityLabel={`Activity: ${summary.currentStreak} day streak, ${compactTokens(summary.tokens)} tokens in 30 days. Open usage.`}
      style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
    >
      <View style={styles.text}>
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>ACTIVITY · 30 DAYS</Text>
        <Text style={[styles.value, tabularNums, { color: tokens.ink, fontFamily: monoFamily.bold }]}>{compactTokens(summary.tokens)} tokens</Text>
        <Text style={[type.monoMeta, { color: tokens.ink3 }]}>{summary.currentStreak}d streak · longest {summary.longestStreak}d</Text>
      </View>
      <Svg width={WEEKS * (CELL + GAP) - GAP} height={7 * (CELL + GAP) - GAP}>
        {weeks.map((column, x) => column.map((cell, y) => cell ? <Rect key={cell.day} x={x * (CELL + GAP)} y={y * (CELL + GAP)} width={CELL} height={CELL} rx={1.5} fill={levels[cell.level]} /> : null))}
      </Svg>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  card: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12, borderWidth: 1, borderRadius: radii.radius4, padding: space.space12, marginBottom: space.space12 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  value: { fontSize: 20, lineHeight: 26 },
});
