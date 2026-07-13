import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { Badge } from "../components/ds/Badge";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BoundedList } from "../components/ds/BoundedList";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { type ModelRow } from "../lib/api";
import { useModels } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

type ModelListItem =
  | { kind: "provider"; provider: string; count: number }
  | { kind: "model"; provider: string; model: ModelRow };

const retryLabel = (until: number) => `retry ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(until * 1000))}`;

const ModelCard = memo(function ModelCard({ model }: { model: ModelRow }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  return (
    <Pressable onPress={() => setOpen((value) => !value)} accessibilityRole="button" accessibilityLabel={`${model.name}, ${model.health ? "benched" : "healthy"}`}>
      <Card style={styles.model}>
        <View style={styles.row}>
          <View style={styles.name}>
            <Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{model.name}</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={1}>{model.id}</Text>
          </View>
          {model.health ? <Badge label="benched" tone="danger" /> : <Badge label="ready" tone="success" />}
        </View>
        <View style={styles.tags}>
          {model.subscription ? <Badge label="subscription" tone="accent" /> : null}
          {model.free ? <Badge label="free" tone="success" /> : null}
          {model.paid ? <Badge label={model.estimated_cost_usd > 0 ? `~$${model.estimated_cost_usd.toFixed(2)}` : "metered"} tone="neutral" /> : null}
          {model.frontier ? <Badge label="frontier" tone="warn" /> : null}
        </View>
        {model.health && open ? <Text style={[type.sub, { color: tokens.danger }]}>{model.health.reason} · {retryLabel(model.health.until_epoch)}</Text> : null}
      </Card>
    </Pressable>
  );
});

const ProviderHeader = memo(function ProviderHeader({ provider, count }: { provider: string; count: number }) {
  return <SectionHeader>{`${provider} · ${count} models`}</SectionHeader>;
});

export default function ModelsScreen() {
  const tokens = useTokens();
  const query = useModels();
  const items = useMemo<ModelListItem[]>(
    () => (query.data?.providers ?? []).flatMap(({ provider, models }) => [
      { kind: "provider" as const, provider, count: models.length },
      ...models.map((model) => ({ kind: "model" as const, provider, model })),
    ]),
    [query.data?.providers],
  );
  const renderItem = useCallback(({ item }: { item: ModelListItem; index: number }) => (
    item.kind === "provider" ? <ProviderHeader provider={item.provider} count={item.count} /> : <ModelCard model={item.model} />
  ), []);
  const keyExtractor = useCallback((item: ModelListItem) => item.kind === "provider" ? `provider:${item.provider}` : `model:${item.provider}:${item.model.id}`, []);
  const { refetch } = query;
  const refresh = useCallback(() => void refetch(), [refetch]);
  const header = useMemo(() => (
    <View style={styles.header}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Models & mesh health</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>Your discovered catalog and failover protection.</Text>
      {query.isError && items.length > 0 ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not refresh model health. Showing saved results.</Text></Card> : null}
    </View>
  ), [items.length, query.isError, tokens]);
  const empty = query.isLoading ? (
    <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text></View>
  ) : query.isError ? (
    <EmptyState icon={Cpu} message="Could not load model health. Pull to retry." />
  ) : query.data?.catalog === "unavailable" ? (
    <EmptyState icon={Cpu} message="No recent model catalog. Run forge models on this host to discover providers." />
  ) : (
    <EmptyState icon={Cpu} message="No models are available from this server." />
  );

  return (
    <DesktopDrillDown>
      <Screen scroll={false} contentContainerStyle={styles.screen}>
      <BoundedList
        data={items}
        renderItem={renderItem}
        keyExtractor={keyExtractor}
        ListHeaderComponent={header}
        ListEmptyComponent={empty}
        refreshing={query.isRefetching}
        onRefresh={refresh}
        contentContainerStyle={styles.content}
      />
      </Screen>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  screen: { paddingHorizontal: 0 },
  content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 },
  header: { gap: space.space12, marginBottom: space.space4 },
  loading: { alignItems: "center", padding: space.space32, gap: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  model: { gap: space.space8, marginBottom: space.space8 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1, gap: 2 },
  tags: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 },
});
