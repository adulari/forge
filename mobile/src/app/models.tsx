import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { Badge } from "../components/ds/Badge";
import { BoundedList } from "../components/ds/BoundedList";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { SearchField } from "../components/ds/SearchField";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { type ModelRow } from "../lib/api";
import { useModels } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

const TIER_ORDER = ["complex", "standard", "trivial"] as const;
type Item = { kind: "tier"; tier: string; count: number } | { kind: "model"; model: ModelRow };
const retryLabel = (until: number) => `retry ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(until * 1000))}`;

const ModelCard = memo(function ModelCard({ model }: { model: ModelRow }) { const tokens = useTokens(); const [open, setOpen] = useState(false); return <Pressable onPress={() => setOpen((value) => !value)} accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={`${model.name}, ${model.tier} tier, ${model.health ? "benched" : "ready"}`}><Card style={styles.model}><View style={styles.row}><View style={styles.name}><Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{model.name}</Text><Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={1}>{model.id}</Text></View>{model.health ? <Badge label="benched" tone="danger" /> : <Badge label="ready" tone="success" />}</View><View style={styles.tags}><Badge label={model.tier} tone={model.tier === "complex" ? "warn" : model.tier === "standard" ? "accent" : "neutral"} />{model.free ? <Badge label="free" tone="success" /> : null}{model.paid ? <Badge label={model.estimated_cost_usd > 0 ? `~$${model.estimated_cost_usd.toFixed(2)}` : "metered"} tone="neutral" /> : null}</View>{model.health && open ? <Text style={[type.sub, { color: tokens.danger }]} numberOfLines={3}>{model.health.reason} · {retryLabel(model.health.until_epoch)}</Text> : null}</Card></Pressable>; });

export default function ModelsScreen() {
  const tokens = useTokens(); const query = useModels(); const [search, setSearch] = useState("");
  const normalized = search.trim().toLocaleLowerCase();
  const models = useMemo(() => (query.data?.providers ?? []).flatMap(({ models }) => models).filter((model) => !normalized || `${model.name} ${model.id} ${model.tier}`.toLocaleLowerCase().includes(normalized)).sort((a, b) => TIER_ORDER.indexOf(a.tier) - TIER_ORDER.indexOf(b.tier) || a.name.localeCompare(b.name)), [query.data?.providers, normalized]);
  const items = useMemo<Item[]>(() => TIER_ORDER.flatMap((tier) => { const rows = models.filter((model) => model.tier === tier); return rows.length ? [{ kind: "tier" as const, tier, count: rows.length }, ...rows.map((model) => ({ kind: "model" as const, model }))] : []; }), [models]);
  const renderItem = useCallback(({ item }: { item: Item; index: number }) => item.kind === "tier" ? <SectionHeader>{`${item.tier} · ${item.count} models`}</SectionHeader> : <ModelCard model={item.model} />, []);
  const header = <View style={styles.header}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Models & mesh health</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Searchable catalog, ranked from complex to trivial work.</Text><SearchField value={search} onChangeText={setSearch} placeholder="Search models" accessibilityLabel="Search models" /></View>;
  const empty = query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text></View> : query.isError ? <EmptyState icon={Cpu} message="Could not load model health. Pull to retry." /> : query.data?.catalog === "unavailable" ? <EmptyState icon={Cpu} message="No recent model catalog. Run forge models on this host to discover providers." /> : <EmptyState icon={Cpu} message={search ? "No models match that search." : "No models are available from this server."} />;
  return <DesktopDrillDown><Screen scroll={false} contentContainerStyle={styles.screen}><BoundedList data={items} renderItem={renderItem} keyExtractor={(item) => item.kind === "tier" ? `tier:${item.tier}` : item.model.id} ListHeaderComponent={header} ListEmptyComponent={empty} refreshing={query.isRefetching} onRefresh={() => void query.refetch()} contentContainerStyle={styles.content} /></Screen></DesktopDrillDown>;
}
const styles = StyleSheet.create({ screen: { paddingHorizontal: 0 }, content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 }, header: { gap: space.space12, marginBottom: space.space4 }, loading: { alignItems: "center", padding: space.space32, gap: space.space12 }, model: { gap: space.space8, marginBottom: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1, gap: 2 }, tags: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 } });
