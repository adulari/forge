import { Cpu } from "lucide-react-native";
import React, { memo, useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BoundedList } from "../components/ds/BoundedList";
import { EmptyState } from "../components/ds/EmptyState";
import { SearchField } from "../components/ds/SearchField";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { type ModelRow } from "../lib/api";
import { useModels } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const TIER_ORDER = ["complex", "standard", "trivial", "all"] as const;
type Tier = (typeof TIER_ORDER)[number];
type ModelListItem = { kind: "tier"; tier: Tier; count: number } | { kind: "model"; model: ModelRow };
const retryLabel = (until: number) => `retry ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(until * 1000))}`;
const tierFor = (model: ModelRow): Tier => model.tier === "complex" || model.tier === "standard" || model.tier === "trivial" ? model.tier : "all";
const contextLabel = (value: number | null | undefined) => value == null ? null : `${new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(value)} context`;

const ModelRowItem = memo(function ModelRowItem({ model }: { model: ModelRow }) {
  const tokens = useTokens(); const [open, setOpen] = useState(false); const benchmark = model.benchmark_intelligence == null ? null : `IQ ${model.benchmark_intelligence.toFixed(1)} · code ${model.benchmark_coding?.toFixed(1) ?? "—"}`;
  const ready = !model.health;
  return <Pressable onPress={() => setOpen((value) => !value)} accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={`${model.name}, ${tierFor(model)} tier, ${ready ? "ready" : "benched"}`}><View style={[styles.model, { borderBottomColor: tokens.hairline }]}><View style={styles.row}><Text style={[styles.name, { color: ready ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{model.name}</Text><View style={[styles.dot, { backgroundColor: ready ? tokens.success : tokens.danger }]} /><Text style={[type.meta, { color: ready ? tokens.success : tokens.danger }]}>{ready ? "ready" : "benched"}</Text></View><View style={styles.metaRow}><Text style={[type.monoMeta, tabularNums, styles.tags, { color: tokens.ink4 }]} numberOfLines={1}>{[model.id, model.subscription ? "subscription" : model.free ? "free" : "api", model.frontier ? "frontier" : tierFor(model)].join(" · ")}</Text>{benchmark || contextLabel(model.context_window) ? <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>{[benchmark, contextLabel(model.context_window)].filter(Boolean).join(" · ")}</Text> : null}</View>{model.health && open ? <Text style={[type.sub, styles.healthReason, { color: tokens.danger }]} numberOfLines={3}>{model.health.reason} · {retryLabel(model.health.until_epoch)}</Text> : null}</View></Pressable>;
});

function ModelsScreenBody() {
  const tokens = useTokens(); const query = useModels(); const [search, setSearch] = useState(""); const needle = search.trim().toLocaleLowerCase();
  const allServerModels = useMemo(() => (query.data?.providers ?? []).flatMap(({ models }) => models), [query.data?.providers]);
  const models = useMemo(() => allServerModels.filter((model) => !needle || `${model.name} ${model.id} ${tierFor(model)}`.toLocaleLowerCase().includes(needle)).sort((a, b) => TIER_ORDER.indexOf(tierFor(a)) - TIER_ORDER.indexOf(tierFor(b)) || (b.benchmark_intelligence ?? -Infinity) - (a.benchmark_intelligence ?? -Infinity) || a.name.localeCompare(b.name)), [allServerModels, needle]);
  const items = useMemo<ModelListItem[]>(() => TIER_ORDER.flatMap((tier) => { const rows = models.filter((model) => tierFor(model) === tier); return rows.length ? [{ kind: "tier" as const, tier, count: rows.length }, ...rows.map((model) => ({ kind: "model" as const, model }))] : []; }), [models]);
  const renderItem = useCallback(({ item }: { item: ModelListItem; index: number }) => item.kind === "tier" ? <SectionHeader>{`${item.tier === "all" ? "All models" : item.tier} · ${item.count}`}</SectionHeader> : <ModelRowItem model={item.model} />, []);
  const header = <View style={styles.header}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Models & mesh health</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Your discovered catalog, ranked by capability.</Text><SearchField value={search} onChangeText={setSearch} placeholder="Search models" accessibilityLabel="Search models" /></View>;
  const empty = query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text></View> : query.isError ? <EmptyState icon={Cpu} message="Could not load model health. Pull to retry." /> : allServerModels.length > 0 ? <EmptyState icon={Cpu} message={search ? "No models match that search." : "Models are available but could not be organized."} /> : query.data?.catalog === "unavailable" ? <EmptyState icon={Cpu} message="No recent model catalog. Run forge models on this host to discover providers." /> : <EmptyState icon={Cpu} message="No models are available from this server." />;
  return <Screen scroll={false} contentContainerStyle={styles.screen}><BoundedList data={items} renderItem={renderItem} keyExtractor={(item) => item.kind === "tier" ? `tier:${item.tier}` : item.model.id} ListHeaderComponent={header} ListEmptyComponent={empty} refreshing={query.isRefetching} onRefresh={() => void query.refetch()} contentContainerStyle={styles.content} /></Screen>;
}

export default function ModelsScreen() {
  return <DesktopDrillDown><SettingsShell active="models"><ModelsScreenBody /></SettingsShell></DesktopDrillDown>;
}
const styles = StyleSheet.create({ screen: { paddingHorizontal: 0 }, content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 }, header: { gap: space.space12, marginBottom: space.space4 }, loading: { alignItems: "center", padding: space.space32, gap: space.space12 }, model: { paddingVertical: space.space12, borderBottomWidth: StyleSheet.hairlineWidth, gap: 4 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold }, dot: { width: 6, height: 6, borderRadius: 3 }, metaRow: { flexDirection: "row", alignItems: "center", gap: space.space8 }, tags: { flex: 1 }, healthReason: { marginTop: 2 } });
