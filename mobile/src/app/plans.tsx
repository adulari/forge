import { router } from "expo-router";
import React from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Card } from "../components/ds/Card";
import { Screen } from "../components/ds/Screen";
import { usePlans } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function PlansScreen() {
  const tokens = useTokens();
  const query = usePlans();
  return <DesktopDrillDown><Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable><Text style={[type.title, { color: tokens.ink }]}>Plans</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Live proposals awaiting review across your sessions.</Text>{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load plans. Pull to retry.</Text></Card> : null}{query.data?.length === 0 ? <Card><Text style={[type.body, { color: tokens.ink2 }]}>No live plans yet. Proposed work will appear here while a session is waiting for review.</Text></Card> : null}{query.data?.map((plan) => <Pressable key={plan.session_id} onPress={() => router.push(`/session/${plan.session_id}`)} accessibilityRole="button"><Card style={styles.card}><Text style={[type.sub, { color: tokens.accent }]}>{plan.session_title || plan.session_id}</Text><Text style={[type.bodyBold, { color: tokens.ink }]}>{plan.title}</Text>{plan.steps.map((step, index) => <View key={`${step.title}-${index}`} style={styles.step}><Text style={[type.sub, { color: tokens.ink3 }]}>{index + 1}</Text><View style={styles.stepBody}><Text style={[type.body, { color: tokens.ink }]}>{step.title}</Text>{step.detail ? <Text style={[type.sub, { color: tokens.ink3 }]}>{step.detail}</Text> : null}</View></View>)}{plan.notes ? <Text style={[type.sub, { color: tokens.ink2 }]}>{plan.notes}</Text> : null}</Card></Pressable>)}</Screen></DesktopDrillDown>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, card: { gap: space.space8 }, step: { flexDirection: "row", gap: space.space8 }, stepBody: { flex: 1, gap: 2 } });
