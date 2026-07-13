import { Zap } from "lucide-react-native";
import React from "react";
import { RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { useHooks } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function HooksScreen() {
  const tokens = useTokens();
  const query = useHooks();
  return <DesktopDrillDown><Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Hooks</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Automations that run around Forge session and tool events.</Text>{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load hooks. Pull to retry.</Text></Card> : null}{query.data?.length === 0 ? <EmptyState icon={Zap} message="No hooks configured." /> : null}{query.data?.map((hook, index) => <Card key={`${hook.event}-${hook.command}-${index}`} style={styles.hook}><View style={styles.row}><Text style={[type.bodyBold, styles.event, { color: tokens.ink }]}>{hook.event}</Text>{hook.cc_compat ? <Badge label="Claude-compatible" tone="accent" /> : null}</View><Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={2}>{hook.command}</Text><View style={styles.meta}><Text style={[type.meta, { color: tokens.ink3 }]}>{hook.matcher ? `matches ${hook.matcher}` : "all matching events"}</Text><Text style={[type.meta, { color: tokens.ink3 }]}>{hook.timeout_secs}s timeout</Text></View></Card>)}</Screen></DesktopDrillDown>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, hook: { gap: space.space8 }, row: { flexDirection: "row", gap: space.space8, alignItems: "center" }, event: { flex: 1 }, meta: { flexDirection: "row", justifyContent: "space-between", gap: space.space8 } });
