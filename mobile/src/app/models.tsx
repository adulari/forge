import { Cpu, Eye, EyeOff, Star } from "lucide-react-native";
import { PageHeader } from "../components/ds/PageHeader";
import React, { memo, useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BoundedList } from "../components/ds/BoundedList";
import { EmptyState } from "../components/ds/EmptyState";
import { SearchField } from "../components/ds/SearchField";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { type ModelRow } from "../lib/api";
import { catalogModels, runtimeLabel } from "../lib/modelCatalog";
import { useModels, useRefreshModels, useUpdateModel } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const TIER_ORDER = ["complex", "standard", "trivial", "all"] as const;
type Tier = (typeof TIER_ORDER)[number];
type ProviderModel = { provider: string; model: ModelRow };
type ModelListItem = { kind: "tier"; tier: Tier; count: number } | { kind: "model"; row: ProviderModel } | { kind: "hidden"; entry: string };
const retryLabel = (until: number) => `retry ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(until * 1000))}`;
const tierFor = (model: ModelRow): Tier => model.tier === "complex" || model.tier === "standard" || model.tier === "trivial" ? model.tier : "all";
const contextLabel = (value: number | null | undefined) => value == null ? null : new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);

const ModelRowItem = memo(function ModelRowItem({ row, favorite, onFavorite, onHide }: { row: ProviderModel; favorite: boolean; onFavorite: () => void; onHide: () => void }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const { provider, model } = row;
  const ready = !model.health;
  // Prototype meta line (mobile.dc.html Models): LEFT `provider · access [· frontier]`,
  // RIGHT `IQ 74.2 · code 76.8 · 200k` — never the full model id (that's what made rows
  // truncate into unreadable "…" soup).
  const access = model.subscription ? "subscription" : model.free ? "free" : "api";
  const left = [provider, runtimeLabel(row).toLowerCase(), access === "free" ? "free" : null, model.frontier ? "frontier" : null].filter(Boolean).join(" · ");
  const benchmark = model.benchmark_intelligence == null ? null : `IQ ${model.benchmark_intelligence.toFixed(1)} · code ${model.benchmark_coding?.toFixed(1) ?? "—"}`;
  const right = [benchmark, contextLabel(model.context_window)].filter(Boolean).join(" · ");
  const actions = <View style={styles.actions}><Pressable onPress={onFavorite} accessibilityRole="button" accessibilityLabel={`${favorite ? "Unfavourite" : "Favourite"} ${model.name}`} accessibilityState={{ selected: favorite }} hitSlop={6} style={styles.action}><Star size={16} strokeWidth={1.75} color={favorite ? tokens.accent : tokens.ink3} fill={favorite ? tokens.accent : "none"} /></Pressable><Pressable onPress={onHide} accessibilityRole="button" accessibilityLabel={`Hide ${model.name}`} hitSlop={6} style={styles.action}><EyeOff size={16} strokeWidth={1.75} color={tokens.ink3} /></Pressable></View>;
  return <View style={styles.modelLine}><Pressable style={styles.modelMain} onPress={() => setOpen((value) => !value)} accessibilityRole="button" accessibilityState={{ expanded: open }} accessibilityLabel={`${model.name}, ${provider}, ${tierFor(model)} tier, ${ready ? "ready" : "benched"}`}><View style={[styles.model, { borderBottomColor: tokens.hairline }]}><View style={styles.row}><Text style={[styles.name, { color: ready ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{model.name}</Text><View style={[styles.dot, { backgroundColor: ready ? tokens.success : tokens.danger }]} /><Text style={[type.meta, { color: ready ? tokens.success : tokens.danger }]}>{ready ? "ready" : "benched"}</Text></View><View style={styles.metaRow}><Text style={[type.monoMeta, tabularNums, styles.tags, { color: tokens.ink4 }]} numberOfLines={1}>{left}</Text>{right ? <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>{right}</Text> : null}</View>{model.health ? <Text style={[type.sub, styles.healthReason, { color: tokens.danger }]} numberOfLines={open ? 6 : 1}>{model.health.reason} · {retryLabel(model.health.until_epoch)}</Text> : null}</View></Pressable>{actions}</View>;
});

function HiddenRow({ entry, onRestore }: { entry: string; onRestore: () => void }) {
  const tokens = useTokens();
  return <View style={[styles.modelLine, styles.hiddenRow, { borderBottomColor: tokens.hairline }]}><Text style={[type.monoMeta, styles.tags, { color: tokens.ink3 }]} numberOfLines={1}>{`hidden · ${entry}`}</Text><Pressable onPress={onRestore} accessibilityRole="button" accessibilityLabel={`Restore ${entry}`} hitSlop={6} style={styles.action}><Eye size={16} strokeWidth={1.75} color={tokens.ink3} /></Pressable></View>;
}

function ModelsScreenBody() {
  const tokens = useTokens(); const query = useModels(); const update = useUpdateModel(); const refresh = useRefreshModels(); const [search, setSearch] = useState(""); const needle = search.trim().toLocaleLowerCase();
  const allServerModels = useMemo(() => catalogModels(query.data), [query.data]);
  const models = useMemo(() => allServerModels.filter(({ provider, model }) => !needle || `${model.name} ${model.id} ${provider} ${tierFor(model)}`.toLocaleLowerCase().includes(needle)).sort((a, b) => TIER_ORDER.indexOf(tierFor(a.model)) - TIER_ORDER.indexOf(tierFor(b.model)) || (b.model.benchmark_intelligence ?? -Infinity) - (a.model.benchmark_intelligence ?? -Infinity) || a.model.name.localeCompare(b.model.name)), [allServerModels, needle]);
  const favorites = useMemo(() => query.data?.favorites ?? [], [query.data?.favorites]);
  const hiddenEntries = useMemo(() => (query.data?.disabled ?? []).filter((entry) => !needle || entry.toLocaleLowerCase().includes(needle)), [query.data?.disabled, needle]);
  const items = useMemo<ModelListItem[]>(() => [...TIER_ORDER.flatMap((tier) => { const rows = models.filter(({ model }) => tierFor(model) === tier).sort((a, b) => Number(favorites.includes(b.model.id)) - Number(favorites.includes(a.model.id))); return rows.length ? [{ kind: "tier" as const, tier, count: rows.length }, ...rows.map((row) => ({ kind: "model" as const, row }))] : []; }), ...hiddenEntries.map((entry) => ({ kind: "hidden" as const, entry }))], [models, favorites, hiddenEntries]);
  const renderItem = useCallback(({ item }: { item: ModelListItem; index: number }) => item.kind === "tier" ? <SectionHeader>{`${item.tier === "all" ? "All models" : item.tier} · ${item.count}`}</SectionHeader> : item.kind === "hidden" ? <HiddenRow entry={item.entry} onRestore={() => update.mutate({ model: item.entry, hidden: false })} /> : <ModelRowItem row={item.row} favorite={favorites.includes(item.row.model.id)} onFavorite={() => update.mutate({ model: item.row.model.id, favorite: !favorites.includes(item.row.model.id) })} onHide={() => update.mutate({ model: item.row.model.id, hidden: true })} />, [favorites, update]);
  const header = <View style={styles.header}><PageHeader title="Models & mesh health" subtitle="Your discovered catalog, ranked by capability. IQ and code scores use benchmark data from Artificial Analysis (artificialanalysis.ai)." /><SearchField value={search} onChangeText={setSearch} placeholder="Search models" accessibilityLabel="Search models" /></View>;
  const empty = query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text></View> : query.isError ? <EmptyState icon={Cpu} message={`Could not load model health: ${query.error?.message || "unknown error"}. Pull to retry.`} /> : allServerModels.length > 0 ? <EmptyState icon={Cpu} message={search ? "No models match that search." : "Models are available but could not be organized."} /> : query.data?.catalog === "unavailable" ? <EmptyState icon={Cpu} message="No recent model catalog. Run forge models on this host to discover providers." /> : <EmptyState icon={Cpu} message="No models are available from this server." />;
  return <Screen scroll={false} contentContainerStyle={styles.screen}><BoundedList data={items} renderItem={renderItem} keyExtractor={(item) => item.kind === "tier" ? `tier:${item.tier}` : item.kind === "hidden" ? `hidden:${item.entry}` : item.row.model.id} ListHeaderComponent={header} ListEmptyComponent={empty} refreshing={refresh.isPending} onRefresh={() => refresh.mutate()} contentContainerStyle={styles.content} /></Screen>;
}

export default function ModelsScreen() {
  return <DesktopDrillDown><SettingsShell active="models"><ModelsScreenBody /></SettingsShell></DesktopDrillDown>;
}
const styles = StyleSheet.create({ screen: { paddingHorizontal: 0 }, content: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space32 }, header: { gap: space.space12, marginBottom: space.space4 }, loading: { alignItems: "center", padding: space.space32, gap: space.space12 }, modelLine: { flexDirection: "row", alignItems: "center" }, modelMain: { flex: 1 }, hiddenRow: { paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth }, actions: { flexDirection: "row" }, action: { width: 32, height: 44, alignItems: "center", justifyContent: "center" }, model: { paddingVertical: space.space12, borderBottomWidth: StyleSheet.hairlineWidth, gap: 4 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold }, dot: { width: 6, height: 6, borderRadius: 3 }, metaRow: { flexDirection: "row", alignItems: "center", gap: space.space8 }, tags: { flex: 1 }, healthReason: { marginTop: 2 } });
