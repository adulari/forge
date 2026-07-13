import { Zap } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { useHooks } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function HooksScreen() { const tokens = useTokens(); const query = useHooks(); const [search, setSearch] = useState(""); const hooks = useMemo(() => (query.data ?? []).filter((hook) => !search.trim() || `${hook.event} ${hook.matcher ?? ""} ${hook.command}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())), [query.data, search]); return <DesktopDrillDown><Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Hooks</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Automations that run around Forge session and tool events.</Text><SearchField value={search} onChangeText={setSearch} placeholder="Search hooks" accessibilityLabel="Search hooks" />{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load hooks. Pull to retry.</Text></Card> : null}{!query.isLoading && hooks.length === 0 ? <EmptyState icon={Zap} message={search ? "No hooks match that search." : "No hooks configured."} /> : null}{hooks.map((hook, index) => <Card key={`${hook.event}-${hook.command}-${index}`} style={styles.hook}><View style={styles.row}><Text style={[type.bodyBold, styles.event, { color: tokens.ink }]} numberOfLines={1}>{hook.event}</Text>{hook.cc_compat ? <Badge label="Claude-compatible" tone="accent" /> : null}</View><Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={2}>{hook.command}</Text><View style={styles.meta}><Text style={[type.meta, styles.matcher, { color: tokens.ink3 }]} numberOfLines={1}>{hook.matcher ? `matches ${hook.matcher}` : "all matching events"}</Text><Text style={[type.meta, { color: tokens.ink3 }]}>{hook.timeout_secs}s timeout</Text></View></Card>)}</Screen></DesktopDrillDown>; }
const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, hook: { gap: space.space8 }, row: { flexDirection: "row", gap: space.space8, alignItems: "center" }, event: { flex: 1 }, meta: { flexDirection: "row", alignItems: "center", gap: space.space8 }, matcher: { flex: 1 } });
