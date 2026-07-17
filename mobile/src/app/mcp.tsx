import React, { useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { AddMcpServerSheet } from "../components/mcp/AddMcpServerSheet";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { useMcp } from "../lib/queries";
import { Plug } from "lucide-react-native";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

function McpScreenBody() {
  const tokens = useTokens();
  const query = useMcp();
  const data = query.data;
  const [adding, setAdding] = useState(false);
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
    <View style={styles.headerRow}><BackLink /><View style={styles.flexFill} /><Pressable onPress={() => setAdding(true)} accessibilityRole="button"><Text style={[styles.add, { color: tokens.accent }]}>+ Add</Text></Pressable></View>
    <Text style={[type.title, { color: tokens.ink }]}>MCP servers</Text>
    <Text style={[type.sub, { color: tokens.ink3 }]}>External tools available to Forge. Secrets remain on the host.</Text>
    {query.isError ? <Text style={[type.body, { color: tokens.danger }]}>Could not load MCP servers. Pull to retry.</Text> : null}
    {data?.servers.length === 0 ? <EmptyState icon={Plug} message="No MCP servers configured." /> : null}
    {data?.servers.map((server, index) => <View key={server.name} style={[styles.server, index < data.servers.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
      <View style={[styles.dot, { backgroundColor: server.enabled ? tokens.success : tokens.ink4 }]} />
      <View style={styles.serverBody}>
        <Text style={[styles.name, { color: server.enabled ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{server.name}</Text>
        <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>{[server.transport, server.auth_configured ? "auth configured" : null, server.secret_env_count > 0 ? `${server.secret_env_count} secret ref${server.secret_env_count === 1 ? "" : "s"}` : null].filter(Boolean).join(" · ")}</Text>
      </View>
      <Badge label={server.enabled ? "enabled" : "disabled"} tone={server.enabled ? "success" : "neutral"} />
    </View>)}
    {data ? <Text style={[type.monoMeta, styles.footerMeta, { color: tokens.ink4 }]}>call timeout {data.call_timeout_secs}s · connect timeout {data.connect_timeout_secs}s</Text> : null}
    <AddMcpServerSheet visible={adding} onClose={() => setAdding(false)} />
  </Screen>;
}

export default function McpScreen() {
  return <DesktopDrillDown><SettingsShell active="mcp"><McpScreenBody /></SettingsShell></DesktopDrillDown>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, headerRow: { flexDirection: "row", alignItems: "center" }, flexFill: { flex: 1 }, add: { fontSize: 15, fontWeight: "600" }, server: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12, minHeight: 56 }, dot: { width: 7, height: 7, borderRadius: 4 }, serverBody: { flex: 1, minWidth: 0, gap: 2 }, name: { fontSize: 14, fontFamily: monoFamily.bold }, footerMeta: { paddingTop: space.space4 } });
