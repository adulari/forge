import { router } from "expo-router";
import React from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { Badge } from "../components/ds/Badge";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { useMcp } from "../lib/queries";
import { Plug } from "lucide-react-native";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function McpScreen() {
  const tokens = useTokens();
  const query = useMcp();
  const data = query.data;
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable><Text style={[type.title, { color: tokens.ink }]}>MCP servers</Text><Text style={[type.sub, { color: tokens.ink3 }]}>External tools available to Forge. Secrets remain on the host.</Text>{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load MCP servers. Pull to retry.</Text></Card> : null}{data?.servers.length === 0 ? <EmptyState icon={Plug} message="No MCP servers configured." /> : null}{data?.servers.map((server) => <Card key={server.name} style={styles.server}><View style={styles.row}><Text style={[type.bodyBold, styles.name, { color: tokens.ink }]} numberOfLines={1}>{server.name}</Text><Badge label={server.enabled ? "enabled" : "disabled"} tone={server.enabled ? "success" : "neutral"} /></View><View style={styles.tags}><Badge label={server.transport} tone="outline" />{server.auth_configured ? <Badge label="auth configured" tone="accent" /> : null}{server.secret_env_count > 0 ? <Badge label={`${server.secret_env_count} secret ref${server.secret_env_count === 1 ? "" : "s"}`} tone="neutral" /> : null}</View></Card>)}{data ? <Card><Text style={[type.sub, { color: tokens.ink3 }]}>Call timeout: {data.call_timeout_secs}s · Connect timeout: {data.connect_timeout_secs}s</Text></Card> : null}</Screen>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, server: { gap: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1 }, tags: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 } });
