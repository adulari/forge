import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { UsageRing } from "../components/shell/UsageRing";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { BoundedList } from "../components/ds/BoundedList";
import { Segmented } from "../components/ds/Segmented";
import { type UsageProvider } from "../lib/api";
import { useSessions, useUsage } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const compact = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value).toLowerCase();
const kindTone = (kind: string) => kind === "api" ? "neutral" : "success";
// Quota window names on the wire -> the prototype's short mono labels.
const WINDOW_LABEL: Record<string, string> = { five_hour: "5h", weekly: "week", secondary: "2nd" };
const resetLabel = (resetsAt: number | null) => resetsAt == null ? null : `resets ${new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }).format(new Date(resetsAt * 1000))}`;

type QuotaRow = { kind: string; windowKind: string; status: string; fraction: number | null; resetsAt: number | null };

// Fixed window durations for the two quota kinds the wire actually sends (api.ts UsageQuota;
// "secondary" has no known fixed length, so it's excluded from pace projection).
const WINDOW_DURATION_SEC: Record<string, number> = { five_hour: 5 * 3600, weekly: 7 * 24 * 3600 };

function formatDurationShort(totalSec: number): string {
  const sec = Math.max(0, Math.round(totalSec));
  const days = Math.floor(sec / 86400);
  const hours = Math.floor((sec % 86400) / 3600);
  const minutes = Math.floor((sec % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${Math.max(1, minutes)}m`;
}

/**
 * Pace-dot annotation ("● ≈75% by reset" warn / "● runs out 1d 8h" danger), computed
 * entirely from existing quota fields — fraction, resetsAt, windowKind, status. No new
 * backend data: the server's own "warning"/"exhausted" status gates whether to project at
 * all (rather than this file inventing its own percent threshold), and the projection math
 * (linear extrapolation of the current fraction over the elapsed portion of the window)
 * only ever runs on top of that real signal.
 */
function paceAnnotation(quota: QuotaRow, nowSec: number): { text: string; tone: "warn" | "danger" } | null {
  if (quota.status !== "warning" && quota.status !== "exhausted") return null;
  const duration = WINDOW_DURATION_SEC[quota.windowKind];
  if (!duration || quota.fraction == null || quota.resetsAt == null) return null;
  const remainingSec = quota.resetsAt - nowSec;
  const elapsedSec = duration - remainingSec;
  if (remainingSec <= 0 || elapsedSec <= 0) return null;
  const rate = quota.fraction / elapsedSec;
  if (rate <= 0) return null;
  if (quota.fraction < 1) {
    const secToExhaust = (1 - quota.fraction) / rate;
    if (secToExhaust < remainingSec) {
      return { text: `runs out ${formatDurationShort(secToExhaust)}`, tone: "danger" };
    }
  }
  const projected = rate * duration;
  return { text: `≈${Math.round(Math.min(999, projected * 100))}% by reset`, tone: "warn" };
}
/** One rendered provider block: token usage (when any), quota windows (when any) — a
 * provider with live quota but no recorded tokens still shows, bars visible. */
interface ProviderItem {
  provider: string;
  kind: string;
  usage: UsageProvider | null;
  quotas: QuotaRow[];
}

const ProviderRow = memo(function ProviderRow({ item, showSeparator, nowSec }: { item: ProviderItem; showSeparator: boolean; nowSec: number }) {
  const tokens = useTokens();
  const costUsd = item.usage?.costUsd ?? 0;
  const priceLabel = costUsd > 0 ? `$${costUsd.toFixed(2)}` : item.kind === "api" ? "No price" : "Included";
  return (
    <View style={[styles.provider, showSeparator ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
      <View style={styles.row}>
        <Text style={[styles.providerName, { color: tokens.ink }]} numberOfLines={1}>{item.provider}</Text>
        <Badge label={item.kind} tone={kindTone(item.kind) as never} lowercase />
        <Text style={[styles.price, tabularNums, { color: costUsd > 0 ? tokens.success : tokens.ink3 }]}>{priceLabel}</Text>
      </View>
      {item.usage ? (
        <Text style={[styles.detail, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>
          {compact(item.usage.inputTokens)} in · {compact(item.usage.outputTokens)} out
        </Text>
      ) : null}
      {item.quotas.map((quota) => {
        const pct = quota.fraction == null ? null : Math.round(quota.fraction * 100);
        const barColor = quota.status === "exhausted" ? tokens.danger : quota.status === "warning" ? tokens.warn : tokens.accent;
        const right = [pct == null ? "—" : `${pct}%`, resetLabel(quota.resetsAt)].filter(Boolean).join(" · ");
        const pace = paceAnnotation(quota, nowSec);
        return (
          <View key={quota.windowKind}>
            <View style={styles.quota} accessibilityLabel={`${WINDOW_LABEL[quota.windowKind] ?? quota.windowKind} window ${right}`}>
              <Text style={[styles.quotaLabel, tabularNums, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>{WINDOW_LABEL[quota.windowKind] ?? quota.windowKind}</Text>
              <View style={[styles.track, { backgroundColor: tokens.border }]}>
                <View style={[styles.fill, { width: `${Math.min(100, Math.max(0, (quota.fraction ?? 0) * 100))}%`, backgroundColor: barColor }]} />
              </View>
              <Text style={[styles.quotaPct, tabularNums, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>{right}</Text>
            </View>
            {pace ? (
              <View style={styles.paceRow}>
                <View style={[styles.paceDot, { backgroundColor: pace.tone === "danger" ? tokens.danger : tokens.warn }]} />
                <Text style={[type.monoMeta, tabularNums, { color: pace.tone === "danger" ? tokens.danger : tokens.warn }]}>{pace.text}</Text>
              </View>
            ) : null}
          </View>
        );
      })}
    </View>
  );
});

function UsageScreenBody() {
  const tokens = useTokens();
  const [window, setWindow] = useState<"week" | "session">("week");
  const { data: sessions } = useSessions();
  const sessionId = sessions?.find((session) => session.busy)?.id ?? sessions?.[0]?.id;
  const query = useUsage(sessionId);
  const selected = window === "week" ? query.data?.week : query.data?.session;
  const { isError, isLoading, isRefetching, refetch, data, dataUpdatedAt } = query;
  // Pace projection is relative to the instant this quota snapshot was fetched, not to
  // whatever moment a render happens to run in — sidesteps calling Date.now() during
  // render (react-hooks/purity) and is arguably more correct: the fraction/resetsAt pair
  // is only accurate as of `dataUpdatedAt` anyway.
  const nowSec = dataUpdatedAt / 1000;
  const quotaRows = data?.quota;
  // Merge token usage + quota windows into one provider list: a provider with a live quota
  // window but zero recorded tokens (fresh daemon, subscription bridges) still gets a block —
  // previously the quota data was invisible until usage.providers had entries.
  const providers = useMemo<ProviderItem[]>(() => {
    const result = new Map<string, ProviderItem>();
    const keyOf = (provider: string, kind: string) => `${kind}:${provider}`;
    for (const usage of selected?.providers ?? []) {
      result.set(keyOf(usage.provider, usage.kind), { provider: usage.provider, kind: usage.kind, usage, quotas: [] });
    }
    for (const quota of quotaRows ?? []) {
      const key = keyOf(quota.provider, quota.kind);
      const existing = result.get(key) ?? { provider: quota.provider, kind: quota.kind, usage: null, quotas: [] };
      existing.quotas.push(quota);
      result.set(key, existing);
    }
    return [...result.values()].sort((a, b) => (b.usage?.costUsd ?? 0) - (a.usage?.costUsd ?? 0) || b.quotas.length - a.quotas.length || a.provider.localeCompare(b.provider));
  }, [selected?.providers, quotaRows]);
  const renderItem = useCallback(({ item, index }: { item: ProviderItem; index: number }) => <ProviderRow item={item} showSeparator={index < providers.length - 1} nowSec={nowSec} />, [providers.length, nowSec]);
  const keyExtractor = useCallback((item: ProviderItem) => `${item.kind}:${item.provider}`, []);
  // The binding constraint, not an average: the fullest subscription window is what
  // actually gates the next turn.
  const subscriptionQuotas = (quotaRows ?? []).filter((quota) => quota.kind !== "api" && quota.fraction != null);
  const combinedSubscriptionPercent = window === "week" && subscriptionQuotas.length > 0
    ? Math.round(Math.max(...subscriptionQuotas.map((quota) => quota.fraction ?? 0)) * 100)
    : null;
  const hasMeteredApi = providers.some((item) => item.kind === "api" && item.usage != null);
  const apiCostUsd = providers.reduce((total, item) => total + (item.kind === "api" ? item.usage?.costUsd ?? 0 : 0), 0);
  const usageLabel = hasMeteredApi ? "API spend" : "Subscription usage";
  const totalTokens = (selected?.combined.inputTokens ?? 0) + (selected?.combined.outputTokens ?? 0);
  const header = useMemo(() => (
    <View style={styles.header}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Usage</Text>
      <Text style={[styles.subtitle, { color: tokens.ink3 }]}>A clear read on your Forge consumption.</Text>
      <Segmented options={[{ value: "week", label: "This Week" }, { value: "session", label: "This Session" }]} value={window} onChange={setWindow} />
      {!isLoading && !isError && selected ? <View style={styles.hero}>
        {/* D/M Settings Usage ring (4px stroke, r=18 in a 44 viewBox) — reuses the shell's
            shared UsageRing (components/shell/UsageRing.tsx) rather than a second local
            SVG-ring implementation, so this page's ring and the shell's per-provider rings
            never drift on stroke width or color steps. Combined %-of-quota only — API-spend
            and session-activity states keep the text-only layout, nothing to ring there. */}
        {combinedSubscriptionPercent != null ? (
          <UsageRing pct={combinedSubscriptionPercent} size={44} strokeWidth={4} accessibilityLabel={`${combinedSubscriptionPercent} percent combined subscription usage`} />
        ) : null}
        <View style={styles.heroText}>
          <Text style={[type.section, { color: tokens.ink4 }]}>{combinedSubscriptionPercent != null ? "combined subscription usage" : window === "session" ? "this session" : usageLabel}</Text>
          {combinedSubscriptionPercent != null ? <Text style={[styles.cost, tabularNums, { color: tokens.accent }]}>{combinedSubscriptionPercent}%</Text> : hasMeteredApi ? <Text style={[styles.cost, tabularNums, { color: tokens.accent }]}>{`$${apiCostUsd.toFixed(2)}`}</Text> : <Text style={[styles.included, { color: tokens.accent }]}>{window === "session" ? "Session activity" : "Included with plan"}</Text>}
          <Text style={[styles.tokens, tabularNums, { color: tokens.ink2, fontFamily: monoFamily.regular }]}>{compact(totalTokens)} tokens</Text>
          <Text style={[styles.split, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>{compact(selected.combined.inputTokens)} in · {compact(selected.combined.outputTokens)} out</Text>
        </View>
      </View> : null}
      {isLoading ? <Text style={[styles.empty, { color: tokens.ink3 }]}>Loading usage…</Text> : null}
      {isError ? <Text style={[styles.empty, { color: tokens.danger }]}>Could not load usage. Pull to retry.</Text> : null}
      {window === "session" && !selected && !isLoading && !isError ? <Text style={[styles.empty, { color: tokens.ink2 }]}>No session usage is available yet. Start or open a session to see its activity.</Text> : null}
      {providers.length > 0 ? <Text style={[type.section, styles.providersLabel, { color: tokens.ink4 }]}>providers</Text> : null}
    </View>
  ), [apiCostUsd, combinedSubscriptionPercent, hasMeteredApi, isError, isLoading, providers.length, selected, tokens, totalTokens, usageLabel, window]);
  const empty = isLoading ? <View /> : <EmptyState icon={Cpu} message="No usage yet. Your provider activity will appear here after the first turn." />;

  return (
    <Screen scroll={false} contentContainerStyle={styles.screen}>
      <BoundedList data={providers} renderItem={renderItem} keyExtractor={keyExtractor} ListHeaderComponent={header} ListEmptyComponent={empty} refreshing={isRefetching} onRefresh={() => void refetch()} contentContainerStyle={styles.content} />
    </Screen>
  );
}

export default function UsageScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="usage">
        <UsageScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  screen: { paddingHorizontal: 0 },
  content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 },
  header: { gap: space.space12, marginBottom: space.space4 },
  subtitle: { marginTop: -6 },
  hero: { flexDirection: "row", alignItems: "center", gap: space.space16, marginTop: space.space8, paddingVertical: space.space8 },
  heroText: { flex: 1, minWidth: 0 },
  cost: { fontSize: 44, lineHeight: 48, fontFamily: monoFamily.bold, marginTop: 8 },
  included: { fontSize: 22, fontWeight: "700", marginTop: 12 },
  tokens: { fontSize: 14, marginTop: 4 },
  split: { fontSize: 12, marginTop: 4 },
  providersLabel: { marginTop: space.space8 },
  provider: { paddingVertical: space.space12, gap: 4 },
  row: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: space.space8 },
  providerName: { flex: 1, fontSize: 15.5, fontWeight: "600" },
  price: { fontSize: 13, fontWeight: "600", flexShrink: 0 },
  detail: { fontSize: 11.5 },
  quota: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: 8 },
  quotaLabel: { fontSize: 11, width: 34 },
  quotaPct: { fontSize: 11, flexShrink: 0 },
  track: { flex: 1, height: 3, borderRadius: 2, overflow: "hidden" },
  fill: { height: "100%", borderRadius: 2 },
  paceRow: { flexDirection: "row", alignItems: "center", gap: space.space4, marginTop: 3, paddingLeft: 42 },
  paceDot: { width: 5, height: 5, borderRadius: 2.5 },
  empty: { fontSize: 15, lineHeight: 21 },
});
