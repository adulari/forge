import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Badge } from "../components/ds/Badge";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BoundedList } from "../components/ds/BoundedList";
import { BackLink } from "../components/ds/BackLink";
import { Card } from "../components/ds/Card";
import { Screen } from "../components/ds/Screen";
import { EmptyState } from "../components/ds/EmptyState";
import { Segmented } from "../components/ds/Segmented";
import { type UsageProvider } from "../lib/api";
import { useSessions, useUsage } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

const number = (value: number) => new Intl.NumberFormat().format(value);
const kindTone = (kind: string) => kind === "bridge" ? "accent" : kind === "oauth" ? "success" : "neutral";
const resetLabel = (resetsAt: number | null) => resetsAt == null ? "reset unknown" : `resets ${new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }).format(new Date(resetsAt * 1000))}`;

interface ProviderRowProps {
  provider: UsageProvider;
  quotas: { kind: string; windowKind: string; status: string; fraction: number | null; resetsAt: number | null }[];
}

const ProviderRow = memo(function ProviderRow({ provider, quotas }: ProviderRowProps) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const priceLabel = provider.costUsd > 0
    ? `$${provider.costUsd.toFixed(2)}`
    : provider.kind === "api" ? "No estimate" : "Included with plan";
  return (
    <Pressable onPress={() => setOpen((value) => !value)} accessibilityRole="button">
      <Card style={styles.provider}>
        <View style={styles.row}>
          <Text style={[styles.providerName, { color: tokens.ink }]} numberOfLines={1}>{provider.provider}</Text>
          <Badge label={provider.kind} tone={kindTone(provider.kind) as never} />
        </View>
        <Text style={[styles.costSmall, { color: tokens.accent }]}>{priceLabel}</Text>
        <Text style={[styles.detail, { color: tokens.ink3 }]}>{number(provider.inputTokens)} in · {number(provider.outputTokens)} out · {open ? "tap to collapse" : "tap for quota details"}</Text>
        {open ? quotas.map((quota) => (
          <View key={quota.windowKind} style={styles.quota}>
            <View style={styles.row}>
              <Text style={[styles.detail, { color: tokens.ink2 }]}>{quota.windowKind}</Text>
              <Text style={[styles.detail, { color: tokens.ink3 }]}>{quota.status} · {quota.fraction == null ? "—" : `${Math.round(quota.fraction * 100)}%`}</Text>
            </View>
            <Text style={[styles.reset, { color: tokens.ink3 }]}>{resetLabel(quota.resetsAt)}</Text>
            <View style={[styles.track, { backgroundColor: tokens.bg3 }]}>
              <View style={[styles.fill, { width: `${Math.min(100, Math.max(0, (quota.fraction ?? 0) * 100))}%`, backgroundColor: quota.status === "exhausted" ? tokens.danger : quota.status === "warning" ? tokens.warn : tokens.accent }]} />
            </View>
          </View>
        )) : null}
      </Card>
    </Pressable>
  );
});

export default function UsageScreen() {
  const tokens = useTokens();
  const [window, setWindow] = useState<"week" | "session">("week");
  const { data: sessions } = useSessions();
  const sessionId = sessions?.find((session) => session.busy)?.id ?? sessions?.[0]?.id;
  const query = useUsage(sessionId);
  const selected = window === "week" ? query.data?.week : query.data?.session;
  const providers = selected?.providers ?? [];
  const { isError, isLoading, isRefetching, refetch, data } = query;
  const quotaRows = data?.quota;
  const quotaKey = (provider: string, kind: string) => `${kind}:${provider}`;
  const quotasByProvider = useMemo(() => {
    const result = new Map<string, NonNullable<typeof data>["quota"]>();
    for (const quota of quotaRows ?? []) {
      const rows = result.get(quotaKey(quota.provider, quota.kind)) ?? [];
      rows.push(quota);
      result.set(quotaKey(quota.provider, quota.kind), rows);
    }
    return result;
  }, [quotaRows]);
  const renderItem = useCallback(({ item }: { item: UsageProvider; index: number }) => <ProviderRow provider={item} quotas={quotasByProvider.get(quotaKey(item.provider, item.kind)) ?? []} />, [quotasByProvider]);
  const keyExtractor = useCallback((provider: UsageProvider) => provider.provider, []);
  const hasMeteredApi = providers.some((provider) => provider.kind === "api");
  const apiCostUsd = providers
    .filter((provider) => provider.kind === "api")
    .reduce((total, provider) => total + provider.costUsd, 0);
  const usageLabel = hasMeteredApi ? "API SPEND" : "SUBSCRIPTION USAGE";
  const totalTokens = (selected?.combined.inputTokens ?? 0) + (selected?.combined.outputTokens ?? 0);
  const header = useMemo(() => (
    <View style={styles.header}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Usage</Text>
      <Text style={[styles.subtitle, { color: tokens.ink3 }]}>A clear read on your Forge consumption.</Text>
      <Segmented options={[{ value: "week", label: "This Week" }, { value: "session", label: "This Session" }]} value={window} onChange={setWindow} />
      {!isLoading && !isError && selected ? <View style={[styles.hero, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
        <Text style={[styles.eyebrow, { color: tokens.ink3 }]}>{usageLabel} · {window === "week" ? "THIS WEEK" : "THIS SESSION"}</Text>
        {hasMeteredApi ? <Text style={[styles.cost, { color: tokens.accent }]}>{`$${apiCostUsd.toFixed(2)}`}</Text> : <Text style={[styles.included, { color: tokens.accent }]}>Included with plan</Text>}
        <Text style={[styles.tokens, { color: tokens.ink }]}>{number(totalTokens)} tokens</Text>
        <Text style={[styles.split, { color: tokens.ink3 }]}>{number(selected.combined.inputTokens)} in · {number(selected.combined.outputTokens)} out</Text>
      </View> : null}
      {isLoading ? <Card><Text style={[styles.empty, { color: tokens.ink3 }]}>Loading usage…</Text></Card> : null}
      {isError ? <Card><Text style={[styles.empty, { color: tokens.danger }]}>Could not load usage. Pull to retry.</Text></Card> : null}
      {window === "session" && !selected && !isLoading && !isError ? <Card><Text style={[styles.empty, { color: tokens.ink2 }]}>No session usage is available yet. Start or open a session to see its activity.</Text></Card> : null}
    </View>
  ), [apiCostUsd, hasMeteredApi, isError, isLoading, selected, tokens, totalTokens, usageLabel, window]);
  const empty = isLoading ? <View /> : <EmptyState icon={Cpu} message="No usage yet. Your provider activity will appear here after the first turn." />;

  return (
    <DesktopDrillDown>
      <Screen scroll={false} contentContainerStyle={styles.screen}>
      <BoundedList data={providers} renderItem={renderItem} keyExtractor={keyExtractor} ListHeaderComponent={header} ListEmptyComponent={empty} refreshing={isRefetching} onRefresh={() => void refetch()} contentContainerStyle={styles.content} />
      </Screen>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  screen: { paddingHorizontal: 0 },
  content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 },
  header: { gap: space.space12, marginBottom: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  subtitle: { marginTop: -6 },
  hero: { borderWidth: 1, borderRadius: 16, padding: 22, marginTop: 8 },
  eyebrow: { fontSize: 11, letterSpacing: 1.4, fontWeight: "700" },
  cost: { fontSize: 42, fontWeight: "800", marginTop: 8 },
  included: { fontSize: 24, fontWeight: "800", marginTop: 12 },
  tokens: { fontSize: 16, fontWeight: "700", marginTop: 2 },
  split: { fontSize: 13, marginTop: 5 },
  provider: { marginBottom: space.space8 },
  row: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: space.space8 },
  providerName: { flex: 1, fontSize: 17, fontWeight: "700" },
  costSmall: { fontSize: 22, fontWeight: "800", marginTop: 10 },
  detail: { fontSize: 13, marginTop: 4 },
  quota: { marginTop: 12 },
  reset: { fontSize: 12, marginTop: 2 },
  track: { height: 7, borderRadius: 4, overflow: "hidden", marginTop: 6 },
  fill: { height: "100%", borderRadius: 4 },
  empty: { lineHeight: 21 },
});
