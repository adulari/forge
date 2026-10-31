import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { BarChart3 } from "lucide-react-native";
import Svg, { Rect } from "react-native-svg";

import type { UsageActivityDay } from "../../lib/api";
import {
  ACTIVITY_WEEKS,
  activityProviders,
  buildHeatmap,
  compactTokens,
  filterProvider,
  summarize,
  type ActivityWindow,
  type HeatCell,
} from "../../lib/usageActivity";
import { canSaveShareImage, shareUsageCard } from "../../lib/usageShareCard";
import { useToday } from "../../lib/useToday";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { EmptyState } from "../ds/EmptyState";
import { Segmented } from "../ds/Segmented";

const CELL = 10;
const GAP = 3;
const WINDOW_LABEL: Record<ActivityWindow, string> = { "7": "Last 7 days", "30": "Last 30 days", all: "Last 52 weeks" };

const dayLabel = (day: string) => {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString("en", { month: "short", day: "numeric", year: "numeric" });
};

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.stat, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]} accessible accessibilityLabel={`${label} ${value}${hint ? `, ${hint}` : ""}`}>
      <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{label}</Text>
      <Text style={[styles.statValue, tabularNums, { color: tokens.ink, fontFamily: monoFamily.bold }]}>{value}</Text>
      {hint ? <Text style={[type.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>{hint}</Text> : null}
    </View>
  );
}

export function ActivityTab({ rows, loading, failed }: { rows: UsageActivityDay[] | undefined; loading: boolean; failed: boolean }) {
  const tokens = useTokens();
  const [window, setWindow] = useState<ActivityWindow>("all");
  const [account, setAccount] = useState<string | null>(null);
  const [picked, setPicked] = useState<HeatCell | null>(null);
  const [shareNote, setShareNote] = useState<string | null>(null);
  const today = useToday(rows);
  const accounts = useMemo(() => activityProviders(rows ?? []), [rows]);
  const filtered = useMemo(() => filterProvider(rows ?? [], account), [rows, account]);
  const weeks = useMemo(() => buildHeatmap(filtered, today, window), [filtered, today, window]);
  const summary = useMemo(() => summarize(filtered, today, window), [filtered, today, window]);
  const levels = useMemo(() => [`${tokens.accent}55`, `${tokens.accent}88`, `${tokens.accent}BB`, tokens.accent] as [string, string, string, string], [tokens.accent]);

  if (loading) return <Text style={[type.sub, { color: tokens.ink3 }]}>Loading activity…</Text>;
  if (failed) return <Text style={[type.sub, { color: tokens.danger }]}>Activity is not available from this host. Update the daemon or connect to it directly.</Text>;
  if (!rows || rows.length === 0) return <EmptyState icon={BarChart3} message="No activity yet. Your daily usage will chart here after the first turn." />;

  const width = ACTIVITY_WEEKS * (CELL + GAP) - GAP;
  const height = 7 * (CELL + GAP) - GAP;
  const share = async () => {
    const result = await shareUsageCard({
      summary,
      weeks,
      windowLabel: WINDOW_LABEL[window],
      accountLabel: account ?? "all accounts",
      palette: { bg: tokens.bg0, ink: tokens.ink, ink3: tokens.ink3, accent: tokens.accent, track: tokens.border, levels },
    });
    setShareNote(result === "failed" ? "Could not create the card." : canSaveShareImage ? "Saved forge-activity.png" : "Shared");
  };

  return (
    <View style={styles.wrap}>
      <Segmented
        options={[{ value: "7", label: "7 days" }, { value: "30", label: "30 days" }, { value: "all", label: "52 weeks" }]}
        value={window}
        onChange={setWindow}
      />
      {accounts.length > 1 ? (
        <View style={styles.chips} accessibilityRole="radiogroup" accessibilityLabel="Account filter">
          <Chip label="All accounts" selected={account == null} onPress={() => setAccount(null)} size="compact" />
          {accounts.map((name) => <Chip key={name} label={name} selected={account === name} onPress={() => setAccount(name)} size="compact" />)}
        </View>
      ) : null}
      <View style={[styles.heatCard, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
        <Svg viewBox={`0 0 ${width} ${height}`} width="100%" height={undefined} style={{ aspectRatio: width / height }} accessibilityLabel={`Activity heatmap, ${summary.activeDays} active days in the ${WINDOW_LABEL[window].toLowerCase()}`}>
          {weeks.map((column, x) => column.map((cell, y) => cell ? (
            <Rect
              key={cell.day}
              x={x * (CELL + GAP)}
              y={y * (CELL + GAP)}
              width={CELL}
              height={CELL}
              rx={2}
              fill={cell.level === 0 || !cell.inWindow ? tokens.border : levels[cell.level - 1]}
              opacity={cell.inWindow ? 1 : 0.35}
              onPress={() => setPicked(cell)}
            />
          ) : null))}
        </Svg>
        <View style={styles.legend}>
          <Text style={[type.monoMeta, tabularNums, { color: tokens.ink3, flex: 1 }]} numberOfLines={1}>
            {picked ? `${dayLabel(picked.day)} · ${compactTokens(picked.tokens)} tokens` : "Tap a day for its total"}
          </Text>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>less</Text>
          {[tokens.border, ...levels].map((color, index) => <View key={index} style={[styles.swatch, { backgroundColor: color }]} />)}
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>more</Text>
        </View>
      </View>
      <View style={styles.stats}>
        <Stat label="TOKENS" value={compactTokens(summary.tokens)} hint={`${compactTokens(summary.calls)} model calls`} />
        <Stat label="ACTIVE DAYS" value={String(summary.activeDays)} hint={summary.busiest ? `busiest ${dayLabel(summary.busiest.day)}` : undefined} />
        <Stat label="STREAK" value={`${summary.currentStreak}d`} hint={`longest ${summary.longestStreak}d`} />
        <Stat label="EST. COST" value={`$${summary.costUsd.toFixed(2)}`} hint="list price, not a bill" />
      </View>
      {summary.providers.length > 0 ? (
        <View style={styles.split}>
          {summary.providers.slice(0, 5).map((item) => (
            <View key={item.provider} style={styles.splitRow}>
              <Text style={[type.sub, { color: tokens.ink2, width: 120 }]} numberOfLines={1}>{item.provider}</Text>
              <View style={[styles.bar, { backgroundColor: tokens.border }]}>
                <View style={[styles.barFill, { backgroundColor: tokens.accent, width: `${Math.max(2, (item.tokens / Math.max(1, summary.tokens)) * 100)}%` }]} />
              </View>
              <Text style={[type.monoMeta, tabularNums, { color: tokens.ink3, width: 52, textAlign: "right" }]}>{compactTokens(item.tokens)}</Text>
            </View>
          ))}
        </View>
      ) : null}
      <View style={styles.shareRow}>
        <Button label={canSaveShareImage ? "Save share card (PNG)" : "Share summary"} variant="secondary" onPress={() => void share()} />
        {shareNote ? <Pressable onPress={() => setShareNote(null)} accessibilityRole="button" accessibilityLabel="Dismiss"><Text style={[type.monoMeta, { color: tokens.ink3 }]}>{shareNote}</Text></Pressable> : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space12 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  heatCard: { borderWidth: 1, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  legend: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  swatch: { width: 10, height: 10, borderRadius: 2 },
  stats: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  stat: { flexGrow: 1, flexBasis: 140, borderWidth: 1, borderRadius: radii.radius4, padding: space.space12, gap: 2 },
  statValue: { fontSize: 24, lineHeight: 30 },
  split: { gap: space.space8 },
  splitRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  bar: { flex: 1, height: 4, borderRadius: 2, overflow: "hidden" },
  barFill: { height: "100%", borderRadius: 2 },
  shareRow: { flexDirection: "row", alignItems: "center", gap: space.space12, flexWrap: "wrap" },
});
