import { router } from "expo-router";
import React, { useMemo, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";
import Animated, { FadeInDown, Layout } from "react-native-reanimated";

import { Badge } from "../components/ds/Badge";
import { Card } from "../components/ds/Card";
import { Screen } from "../components/ds/Screen";
import { Segmented } from "../components/ds/Segmented";
import { useSessions, useUsage } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

const number = (value: number) => new Intl.NumberFormat().format(value);
const kindTone = (kind: string) => kind === "bridge" ? "accent" : kind === "oauth" ? "success" : "neutral";
const resetLabel = (resetsAt: number | null) => resetsAt == null ? "reset unknown" : `resets ${new Intl.DateTimeFormat(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" }).format(new Date(resetsAt * 1000))}`;

export default function UsageScreen() {
  const tokens = useTokens();
  const [window, setWindow] = useState<"week" | "session">("week");
  const [expanded, setExpanded] = useState<string | null>(null);
  const { data: sessions } = useSessions();
  const sessionId = sessions?.find((session) => session.busy)?.id;
  const query = useUsage(sessionId);
  const selected = window === "session" && query.data?.session ? query.data.session : query.data?.week;
  const providers = selected?.providers ?? [];
  const quota = useMemo(() => query.data?.quota ?? [], [query.data?.quota]);
  const refreshing = query.isFetching;
  const totalTokens = (selected?.combined.inputTokens ?? 0) + (selected?.combined.outputTokens ?? 0);
  const quotasByProvider = useMemo(() => new Map(quota.map((item) => [item.provider, quota.filter((q) => q.provider === item.provider)])), [quota]);

  return (
    <Screen scroll refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
      <Pressable onPress={() => router.back()}><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable>
      <Text style={[type.title, { color: tokens.ink }]}>Usage</Text>
      <Text style={[styles.subtitle, { color: tokens.ink3 }]}>A clear read on your Forge consumption.</Text>
      <Segmented options={[{ value: "week", label: "This Week" }, { value: "session", label: "This Session" }]} value={window} onChange={setWindow} />
      <Animated.View entering={FadeInDown.duration(350)} style={[styles.hero, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
        <Text style={[styles.eyebrow, { color: tokens.ink3 }]}>COMBINED {window === "week" ? "WEEKLY" : "SESSION"}</Text>
        <Text style={[styles.cost, { color: tokens.accent }]}>${(selected?.combined.costUsd ?? 0).toFixed(2)}</Text>
        <Text style={[styles.tokens, { color: tokens.ink }]}>{number(totalTokens)} tokens</Text>
        <Text style={[styles.split, { color: tokens.ink3 }]}>{number(selected?.combined.inputTokens ?? 0)} in · {number(selected?.combined.outputTokens ?? 0)} out</Text>
      </Animated.View>
      {providers.length === 0 ? <Card><Text style={[styles.empty, { color: tokens.ink2 }]}>No usage yet. Your provider activity will glow here after the first turn.</Text></Card> : providers.map((provider, index) => {
        const open = expanded === provider.provider;
        return <Animated.View key={provider.provider} entering={FadeInDown.delay(index * 45)} layout={Layout.springify()}>
          <Pressable onPress={() => setExpanded(open ? null : provider.provider)}>
            <Card style={styles.provider}>
              <View style={styles.row}><Text style={[styles.providerName, { color: tokens.ink }]}>{provider.provider}</Text><Badge label={provider.kind} tone={kindTone(provider.kind) as never} /></View>
              <Text style={[styles.costSmall, { color: tokens.accent }]}>${provider.costUsd.toFixed(2)}</Text>
              <Text style={[styles.detail, { color: tokens.ink3 }]}>{number(provider.inputTokens)} in · {number(provider.outputTokens)} out · {open ? "tap to collapse" : "tap for quota details"}</Text>
              {(quotasByProvider.get(provider.provider) ?? []).map((q) => <View key={q.windowKind} style={styles.quota}><View style={styles.row}><Text style={[styles.detail, { color: tokens.ink2 }]}>{q.windowKind}</Text><Text style={[styles.detail, { color: tokens.ink3 }]}>{q.status} · {q.fraction == null ? "—" : `${Math.round(q.fraction * 100)}%`}</Text></View><Text style={[styles.reset, { color: tokens.ink3 }]}>{resetLabel(q.resetsAt)}</Text><View style={[styles.track, { backgroundColor: tokens.bg3 }]}><View style={[styles.fill, { width: `${Math.min(100, Math.max(0, (q.fraction ?? 0) * 100))}%`, backgroundColor: q.status === "exhausted" ? tokens.danger : q.status === "warning" ? tokens.warn : tokens.accent }]} /></View></View>)}
            </Card>
          </Pressable>
        </Animated.View>;
      })}
    </Screen>
  );
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, subtitle: { marginTop: -6 }, hero: { borderWidth: 1, borderRadius: 16, padding: 22, marginTop: 8 }, eyebrow: { fontSize: 11, letterSpacing: 1.4, fontWeight: "700" }, cost: { fontSize: 42, fontWeight: "800", marginTop: 8 }, tokens: { fontSize: 16, fontWeight: "700", marginTop: 2 }, split: { fontSize: 13, marginTop: 5 }, provider: { marginVertical: 2 }, row: { flexDirection: "row", justifyContent: "space-between", alignItems: "center" }, providerName: { fontSize: 17, fontWeight: "700" }, costSmall: { fontSize: 22, fontWeight: "800", marginTop: 10 }, detail: { fontSize: 13, marginTop: 4 }, quota: { marginTop: 12 }, reset: { fontSize: 12, marginTop: 2 }, track: { height: 7, borderRadius: 4, overflow: "hidden", marginTop: 6 }, fill: { height: "100%", borderRadius: 4 }, empty: { lineHeight: 21 },});
