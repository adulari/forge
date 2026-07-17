import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
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
import { type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const number = (value: number) => new Intl.NumberFormat().format(value);
const kindTone = (kind: string) => kind === "bridge" ? "accent" : kind === "oauth" ? "success" : "neutral";
const resetLabel = (resetsAt: number | null) => resetsAt == null ? "reset unknown" : `resets ${new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }).format(new Date(resetsAt * 1000))}`;

interface ProviderRowProps {
  provider: UsageProvider;
  quotas: { kind: string; windowKind: string; status: string; fraction: number | null; resetsAt: number | null }[];
  showSeparator: boolean;
}

const ProviderRow = memo(function ProviderRow({ provider, quotas, showSeparator }: ProviderRowProps) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const priceLabel = provider.costUsd > 0 ? `$${provider.costUsd.toFixed(2)}` : provider.kind === "api" ? "No price" : "Included";
  return (
    <Pressable onPress={() => setOpen((value) => !value)} accessibilityRole="button" accessibilityState={{ expanded: open }}>
      <View style={[styles.provider, showSeparator ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
        <View style={styles.row}>
          <Text style={[styles.providerName, { color: tokens.ink }]} numberOfLines={1}>{provider.provider}</Text>
          <Badge label={provider.kind} tone={kindTone(provider.kind) as never} />
          <Text style={[styles.price, tabularNums, { color: provider.costUsd > 0 ? tokens.accent : tokens.ink3 }]}>{priceLabel}</Text>
        </View>
        <Text style={[styles.detail, tabularNums, { color: tokens.ink4 }]}>{number(provider.inputTokens)} in · {number(provider.outputTokens)} out</Text>
        {open ? quotas.map((quota) => (
          <View key={quota.windowKind} style={styles.quota}>
            <View style={styles.row}>
              <Text style={[styles.detail, { color: tokens.ink2 }]}>{quota.windowKind}</Text>
              <Text style={[styles.detail, tabularNums, { color: tokens.ink3 }]}>{quota.status} · {quota.fraction == null ? "—" : `${Math.round(quota.fraction * 100)}%`}</Text>
            </View>
            <Text style={[styles.reset, { color: tokens.ink3 }]}>{resetLabel(quota.resetsAt)}</Text>
            <View style={[styles.track, { backgroundColor: tokens.bg3 }]}>
              <View style={[styles.fill, { width: `${Math.min(100, Math.max(0, (quota.fraction ?? 0) * 100))}%`, backgroundColor: quota.status === "exhausted" ? tokens.danger : quota.status === "warning" ? tokens.warn : tokens.accent }]} />
            </View>
          </View>
        )) : null}
      </View>
    </Pressable>
  );
});

function UsageScreenBody() {
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
  const renderItem = useCallback(({ item, index }: { item: UsageProvider; index: number }) => <ProviderRow provider={item} quotas={quotasByProvider.get(quotaKey(item.provider, item.kind)) ?? []} showSeparator={index < providers.length - 1} />, [quotasByProvider, providers.length]);
  const keyExtractor = useCallback((provider: UsageProvider) => provider.provider, []);
  const subscriptionQuotas = (quotaRows ?? []).filter((quota) => quota.kind !== "api" && quota.fraction != null);
  const combinedSubscriptionPercent = window === "week" && subscriptionQuotas.length > 0
    ? Math.round(subscriptionQuotas.reduce((total, quota) => total + (quota.fraction ?? 0), 0) * 100 / subscriptionQuotas.length)
    : null;
  const hasMeteredApi = providers.some((provider) => provider.kind === "api");
  const apiCostUsd = providers
    .filter((provider) => provider.kind === "api")
    .reduce((total, provider) => total + provider.costUsd, 0);
  const usageLabel = hasMeteredApi ? "API spend" : "Subscription usage";
  const totalTokens = (selected?.combined.inputTokens ?? 0) + (selected?.combined.outputTokens ?? 0);
  const header = useMemo(() => (
    <View style={styles.header}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Usage</Text>
      <Text style={[styles.subtitle, { color: tokens.ink3 }]}>A clear read on your Forge consumption.</Text>
      <Segmented options={[{ value: "week", label: "This Week" }, { value: "session", label: "This Session" }]} value={window} onChange={setWindow} />
      {!isLoading && !isError && selected ? <View style={styles.hero}>
        <Text style={[type.section, { color: tokens.ink4 }]}>{combinedSubscriptionPercent != null ? "combined subscription usage" : window === "session" ? "this session" : usageLabel}</Text>
        {combinedSubscriptionPercent != null ? <Text style={[styles.cost, tabularNums, { color: tokens.accent }]}>{combinedSubscriptionPercent}%</Text> : hasMeteredApi ? <Text style={[styles.cost, tabularNums, { color: tokens.accent }]}>{`$${apiCostUsd.toFixed(2)}`}</Text> : <Text style={[styles.included, { color: tokens.accent }]}>{window === "session" ? "Session activity" : "Included with plan"}</Text>}
        <Text style={[styles.tokens, tabularNums, { color: tokens.ink }]}>{number(totalTokens)} tokens</Text>
        <Text style={[styles.split, tabularNums, { color: tokens.ink4 }]}>{number(selected.combined.inputTokens)} in · {number(selected.combined.outputTokens)} out</Text>
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
  hero: { marginTop: space.space8, paddingVertical: space.space8 },
  cost: { fontSize: 44, lineHeight: 48, fontWeight: "700", marginTop: 8 },
  included: { fontSize: 22, fontWeight: "700", marginTop: 12 },
  tokens: { fontSize: 14, marginTop: 4 },
  split: { fontSize: 12, marginTop: 4 },
  providersLabel: { marginTop: space.space8 },
  provider: { paddingVertical: space.space12, gap: 4 },
  row: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: space.space8 },
  providerName: { flex: 1, fontSize: 15.5, fontWeight: "600" },
  price: { fontSize: 13, fontWeight: "700", flexShrink: 0 },
  detail: { fontSize: 11.5 },
  quota: { marginTop: 10 },
  reset: { fontSize: 12, marginTop: 2 },
  track: { height: 3, borderRadius: 2, overflow: "hidden", marginTop: 6 },
  fill: { height: "100%", borderRadius: 2 },
  empty: { fontSize: 15, lineHeight: 21 },
});
