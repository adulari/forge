import React from "react";
import { ActivityIndicator, RefreshControl, StyleSheet, Text, View } from "react-native";
import { Sparkles } from "lucide-react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useSkills } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function SkillsScreen() {
  const tokens = useTokens();
  const query = useSkills();
  const groups = new Map<string, NonNullable<typeof query.data>>();
  for (const skill of query.data ?? []) groups.set(skill.scope, [...(groups.get(skill.scope) ?? []), skill]);

  return (
    <DesktopDrillDown>
      <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
        <BackLink />
        <Text style={[type.title, { color: tokens.ink }]}>Skills</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>Reusable Forge methodologies available to every session.</Text>
        {query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading skills…</Text></View> : null}
        {query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load skills. Pull to retry.</Text></Card> : null}
        {!query.isLoading && !query.isError && groups.size === 0 ? <EmptyState icon={Sparkles} message="No skills are available on this server." /> : null}
        {[...groups.entries()].map(([scope, skills]) => <View key={scope}><SectionHeader>{scope}</SectionHeader>{skills.map((skill) => <Card key={skill.name} style={styles.skill}><View style={styles.row}><Text style={[type.body, styles.name, { color: tokens.ink }]} numberOfLines={1}>{skill.name}</Text>{skill.tier ? <Badge label={skill.tier} tone="accent" /> : null}</View><Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={3}>{skill.description}</Text>{skill.resources > 0 ? <Text style={[type.sub, { color: tokens.ink3 }]}>{skill.resources} resource{skill.resources === 1 ? "" : "s"}</Text> : null}</Card>)}</View>)}
      </Screen>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, loading: { alignItems: "center", padding: space.space32, gap: space.space12 }, skill: { gap: space.space4, marginBottom: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1 } });
