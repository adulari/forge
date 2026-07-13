import { router } from "expo-router";
import React, { useMemo, useState } from "react";
import { ActivityIndicator, Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { Badge } from "../components/ds/Badge";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { type ModelRow } from "../lib/api";
import { useModels } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { Cpu } from "lucide-react-native";

const retryLabel = (until: number) => `retry ${new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(until * 1000))}`;

function ModelCard({ model }: { model: ModelRow }) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  return <Pressable onPress={() => setOpen(!open)} accessibilityRole="button" accessibilityLabel={`${model.name}, ${model.health ? "benched" : "healthy"}`}><Card style={styles.model}><View style={styles.row}><View style={styles.name}><Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{model.name}</Text><Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={1}>{model.id}</Text></View>{model.health ? <Badge label="benched" tone="danger" /> : <Badge label="ready" tone="success" />}</View><View style={styles.tags}>{model.subscription ? <Badge label="subscription" tone="accent" /> : null}{model.free ? <Badge label="free" tone="success" /> : null}{model.paid ? <Badge label={model.estimated_cost_usd > 0 ? `~$${model.estimated_cost_usd.toFixed(2)}` : "metered"} tone="neutral" /> : null}{model.frontier ? <Badge label="frontier" tone="warn" /> : null}</View>{model.health && open ? <Text style={[type.sub, { color: tokens.danger }]}>{model.health.reason} · {retryLabel(model.health.until_epoch)}</Text> : null}</Card></Pressable>;
}

export default function ModelsScreen() {
  const tokens = useTokens();
  const query = useModels();
  const providers = useMemo(() => query.data?.providers ?? [], [query.data?.providers]);
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable><Text style={[type.title, { color: tokens.ink }]}>Models & mesh health</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Your discovered catalog and failover protection.</Text>{query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text></View> : null}{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load model health. Pull to retry.</Text></Card> : null}{query.data?.catalog === "unavailable" ? <EmptyState icon={Cpu} message="No recent model catalog. Run forge models on this host to discover providers." /> : null}{providers.map((provider) => <View key={provider.provider}><SectionHeader>{`${provider.provider} · ${provider.models.length} models`}</SectionHeader>{provider.models.map((model) => <ModelCard key={model.id} model={model} />)}</View>)}</Screen>;
}

const styles = StyleSheet.create({ loading: { alignItems: "center", padding: space.space32, gap: space.space12 }, content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, model: { gap: space.space8, marginBottom: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1, gap: 2 }, tags: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 } });
