// Machined desktop shell — the Usage dock (D Split + Usage / W Main, docs/design/
// machined INVENTORY.md): per-provider cards with 5h + weekly quota bars and a pace
// caption, driven from the SAME `useUsage()`/`useSessions()` hooks app/usage.tsx
// renders as its provider list — this is a docked re-flow of that data, not a
// second source of truth.
import { router } from "expo-router";
import { Cpu } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useSessions, useUsage } from "../../lib/queries";
import { buildProviderBlocks, isSubscription, kindLabel, priceLabel, quotaSummary, quotaTiming, windowLabel, type ProviderBlock } from "../../lib/usageWindows";
import { paceAnnotation } from "../../lib/usagePace";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatRelativeTime, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { usagePaceColor } from "./UsageRing";

const compact = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value).toLowerCase();

function ProviderCard({ block, nowSec }: { block: ProviderBlock; nowSec: number }) {
  const tokens = useTokens();
  const price = priceLabel(block);
  const billed = !isSubscription(block.kind) && (block.usage?.costUsd ?? 0) > 0;

  return (
    <View style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
      <View style={styles.cardHeader}>
        <Text style={[typeScale.bodyBold, styles.cardTitle, { color: tokens.ink }]} numberOfLines={1}>
          {block.provider}
        </Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{kindLabel(block.kind)}</Text>
      </View>
      <Text style={[typeScale.monoMeta, tabularNums, { color: billed ? tokens.success : tokens.ink3 }]}>
        {billed ? `${price} (7d)` : price === "Included" ? "Included with plan" : price}
      </Text>
      {block.quotas.map((quota) => {
        const pct = quota.fraction == null ? 0 : Math.max(0, Math.min(100, quota.fraction * 100));
        const barColor = quota.status === "exhausted" ? tokens.danger : usagePaceColor(pct, tokens);
        const detail = quotaSummary(quota, nowSec);
        const pace = quota.seen ? paceAnnotation(quota, nowSec) : null;
        const paceColor = pace?.tone === "danger" ? tokens.danger : tokens.warn;
        return (
          <View key={quota.windowKind} style={styles.quotaBlock} accessibilityLabel={`${windowLabel(quota.windowKind)} ${detail}${pace ? `, ${pace.text}` : ""}`}>
            <View style={styles.quotaHeadRow}>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={1}>{windowLabel(quota.windowKind)}</Text>
              <Text style={[typeScale.monoMeta, tabularNums, { color: quota.seen ? tokens.ink2 : tokens.ink4 }]}>
                {quota.fraction == null ? "unknown" : `${Math.round(pct)}%`}
              </Text>
            </View>
            <View style={[styles.track, { backgroundColor: tokens.border }]}>
              <View style={[styles.fill, { width: `${pct}%`, backgroundColor: barColor }]} />
            </View>
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
              {quota.seen ? quotaTiming(quota, nowSec) || "no reset time reported" : "not seen yet"}
            </Text>
            {pace ? (
              <View style={styles.paceRow}>
                <View style={[styles.paceDot, { backgroundColor: paceColor }]} />
                <Text style={[typeScale.monoMeta, tabularNums, { color: paceColor }]} numberOfLines={1}>{pace.text}</Text>
              </View>
            ) : null}
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

  const blocks = useMemo(() => buildProviderBlocks(week?.providers, quotaRows), [week, quotaRows]);
  const nowSec = query.dataUpdatedAt / 1000;

  return (
    <View style={styles.dock}>
      <View style={styles.list}>
        {query.isLoading && blocks.length === 0 ? (
          <Text style={[typeScale.sub, styles.loadingText, { color: tokens.ink3 }]}>Loading usage…</Text>
        ) : blocks.length === 0 ? (
          <EmptyState icon={Cpu} message="No usage yet — your provider activity will appear here after the first turn." />
        ) : (
          blocks.map((block) => <ProviderCard key={`${block.kind}:${block.provider}`} block={block} nowSec={nowSec} />)
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
  paceRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  paceDot: { width: 6, height: 6, borderRadius: 3 },
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
