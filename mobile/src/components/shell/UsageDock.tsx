// Machined desktop shell — the Usage dock (D Split + Usage / W Main, docs/design/
// machined INVENTORY.md): per-provider cards with 5h + weekly quota bars and a pace
// caption, driven from the SAME `useUsage()`/`useSessions()` hooks app/usage.tsx
// renders as its provider list — this is a docked re-flow of that data, not a
// second source of truth.
import { router } from "expo-router";
import { Cpu } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { type UsageProvider, type UsageQuota } from "../../lib/api";
import { useSessions, useUsage } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, formatRelativeTime, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { usagePaceColor } from "./UsageRing";

const compact = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value).toLowerCase();
const WINDOW_LABEL: Record<string, string> = { five_hour: "Session (5h)", weekly: "Weekly", secondary: "Secondary" };

interface ProviderBlock {
  provider: string;
  kind: string;
  usage: UsageProvider | null;
  quotas: UsageQuota[];
}

function paceCaption(quota: UsageQuota): { text: string; color: "ink3" | "warn" | "danger" } | null {
  if (quota.fraction == null) return null;
  const pct = Math.round(quota.fraction * 100);
  if (quota.status === "exhausted" || pct >= 90) return { text: `● ≈${pct}% — near the limit`, color: "danger" };
  if (pct >= 70) return { text: `● ≈${pct}% by reset`, color: "warn" };
  return { text: `● ≈${pct}% by reset`, color: "ink3" };
}

function ProviderCard({ block }: { block: ProviderBlock }) {
  const tokens = useTokens();
  const costUsd = block.usage?.costUsd ?? 0;

  return (
    <View style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
      <View style={styles.cardHeader}>
        <Text style={[typeScale.bodyBold, styles.cardTitle, { color: tokens.ink }]} numberOfLines={1}>
          {block.provider}
        </Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{block.kind}</Text>
      </View>
      {costUsd > 0 ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]}>{formatCost(costUsd)} today</Text>
      ) : null}
      {block.quotas.map((quota) => {
        const pct = quota.fraction == null ? 0 : Math.max(0, Math.min(100, quota.fraction * 100));
        const pace = paceCaption(quota);
        const barColor = usagePaceColor(pct, tokens);
        return (
          <View key={quota.windowKind} style={styles.quotaBlock}>
            <View style={styles.quotaHeadRow}>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{WINDOW_LABEL[quota.windowKind] ?? quota.windowKind}</Text>
              <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{quota.fraction == null ? "—" : `${Math.round(pct)}%`}</Text>
            </View>
            <View style={[styles.track, { backgroundColor: tokens.border }]}>
              <View style={[styles.fill, { width: `${pct}%`, backgroundColor: barColor }]} />
            </View>
            {pace ? <Text style={[typeScale.monoMeta, { color: tokens[pace.color] }]}>{pace.text}</Text> : null}
          </View>
        );
      })}
      {block.usage ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>
          {compact(block.usage.inputTokens)} in · {compact(block.usage.outputTokens)} out
        </Text>
      ) : null}
    </View>
  );
}

export function UsageDock() {
  const tokens = useTokens();
  const { data: sessions } = useSessions();
  const sessionId = sessions?.find((s) => s.busy)?.id ?? sessions?.[0]?.id;
  const query = useUsage(sessionId);
  const week = query.data?.week;
  const quotaRows = query.data?.quota;

  const blocks = useMemo<ProviderBlock[]>(() => {
    const map = new Map<string, ProviderBlock>();
    const keyOf = (provider: string, kind: string) => `${kind}:${provider}`;
    for (const usage of week?.providers ?? []) {
      map.set(keyOf(usage.provider, usage.kind), { provider: usage.provider, kind: usage.kind, usage, quotas: [] });
    }
    for (const quota of quotaRows ?? []) {
      const key = keyOf(quota.provider, quota.kind);
      const existing = map.get(key) ?? { provider: quota.provider, kind: quota.kind, usage: null, quotas: [] };
      existing.quotas.push(quota);
      map.set(key, existing);
    }
    return [...map.values()].sort((a, b) => (b.usage?.costUsd ?? 0) - (a.usage?.costUsd ?? 0) || a.provider.localeCompare(b.provider));
  }, [week, quotaRows]);

  return (
    <View style={styles.dock}>
      <View style={styles.list}>
        {query.isLoading && blocks.length === 0 ? (
          <Text style={[typeScale.sub, styles.loadingText, { color: tokens.ink3 }]}>Loading usage…</Text>
        ) : blocks.length === 0 ? (
          <EmptyState icon={Cpu} message="No usage yet — your provider activity will appear here after the first turn." />
        ) : (
          blocks.map((block) => <ProviderCard key={`${block.kind}:${block.provider}`} block={block} />)
        )}
        <Pressable
          onPress={() => router.push("/settings")}
          accessibilityRole="button"
          accessibilityLabel="Connect a provider — configure in Settings"
          style={[styles.connectCard, { borderColor: tokens.border }]}
        >
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>+ Connect provider</Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>configure in Settings</Text>
        </Pressable>
      </View>
      <View style={[styles.footer, { borderTopColor: tokens.border }]}>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>
          {query.dataUpdatedAt ? `Updated ${formatRelativeTime(query.dataUpdatedAt)} ago` : "—"}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  dock: { flex: 1 },
  list: { flex: 1, padding: space.space12, gap: space.space8 },
  loadingText: { paddingTop: space.space16, textAlign: "center" },
  card: { borderWidth: 1, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  cardHeader: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  cardTitle: { flex: 1 },
  quotaBlock: { gap: 3 },
  quotaHeadRow: { flexDirection: "row", justifyContent: "space-between" },
  track: { height: 3, borderRadius: 2, overflow: "hidden" },
  fill: { height: "100%", borderRadius: 2 },
  connectCard: {
    borderWidth: 1,
    borderStyle: "dashed",
    borderRadius: radii.radius4,
    padding: space.space12,
    alignItems: "center",
    gap: 2,
  },
  footer: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
  },
});
