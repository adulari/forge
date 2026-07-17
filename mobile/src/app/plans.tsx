import { router } from "expo-router";
import React from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { Screen } from "../components/ds/Screen";
import { usePlans } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type, tabularNums } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

function PlansScreenBody() {
  const tokens = useTokens();
  const query = usePlans();
  const plans = query.data ?? [];
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
    <BackLink />
    <Text style={[type.title, { color: tokens.ink }]}>Plans</Text>
    <Text style={[type.sub, { color: tokens.ink3 }]}>Live proposals awaiting review across your sessions.</Text>
    {query.isError ? <Text style={[type.body, { color: tokens.danger }]}>Could not load plans. Pull to retry.</Text> : null}
    {query.isSuccess && plans.length === 0 ? <Text style={[type.body, { color: tokens.ink2 }]}>No live plans yet. Proposed work will appear here while a session is waiting for review.</Text> : null}
    {plans.map((plan, index) => <Pressable key={plan.session_id} onPress={() => router.push(`/session/${plan.session_id}`)} accessibilityRole="button">
      <View style={[styles.plan, index < plans.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
        <View style={styles.sessionRow}><View style={[styles.tick, { backgroundColor: tokens.accent }]} /><Text style={[type.monoMeta, tabularNums, { color: tokens.accent }]} numberOfLines={1}>{plan.session_title || plan.session_id}</Text></View>
        <Text style={[styles.title, { color: tokens.ink }]}>{plan.title}</Text>
        {plan.steps.map((step, stepIndex) => <View key={`${step.title}-${stepIndex}`} style={styles.step}>
          <Text style={[type.monoMeta, tabularNums, styles.stepIndex, { color: tokens.ink4 }]}>{stepIndex + 1}</Text>
          <View style={styles.stepBody}>
            <Text style={[type.body, { color: tokens.ink }]}>{step.title}</Text>
            {step.detail ? <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{step.detail}</Text> : null}
          </View>
        </View>)}
        {plan.notes ? <Text style={[type.sub, styles.notes, { color: tokens.ink3, borderLeftColor: tokens.border }]}>{plan.notes}</Text> : null}
      </View>
    </Pressable>)}
  </Screen>;
}

export default function PlansScreen() {
  return <DesktopDrillDown><SettingsShell active="plans"><PlansScreenBody /></SettingsShell></DesktopDrillDown>;
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  plan: { paddingVertical: space.space16, gap: space.space8 },
  sessionRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  tick: { width: 6, height: 2 },
  title: { fontSize: 15.5, lineHeight: 22, fontWeight: "600" },
  step: { flexDirection: "row", gap: space.space8 },
  stepIndex: { width: 14, flexShrink: 0, paddingTop: 2 },
  stepBody: { flex: 1, gap: 2 },
  notes: { lineHeight: 19, paddingLeft: space.space16, borderLeftWidth: 2 },
});
