import React, { useMemo, useState } from "react";
import { RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useSkills } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { Sparkles } from "lucide-react-native";
import { SettingsShell } from "./(tabs)/settings";

type SkillRows = NonNullable<ReturnType<typeof useSkills>["data"]>;

function SkillsScreenBody() { const tokens = useTokens(); const query = useSkills(); const skills = query.data; const [search, setSearch] = useState(""); const groups = useMemo(() => { const result = new Map<string, SkillRows>(); for (const skill of skills ?? []) if (!search.trim() || `${skill.name} ${skill.description} ${skill.scope} ${skill.tier ?? ""}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())) result.set(skill.scope, [...(result.get(skill.scope) ?? []), skill]); return result; }, [skills, search]); return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Skills</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Reusable Forge methodologies available to every session.</Text><SearchField value={search} onChangeText={setSearch} placeholder="Search skills" accessibilityLabel="Search skills" />{query.isError ? <Text style={[type.body, { color: tokens.danger }]}>Could not load skills. Pull to retry.</Text> : null}{!query.isLoading && groups.size === 0 ? <EmptyState icon={Sparkles} message={search ? "No skills match that search." : "No skills are available on this server."} /> : null}{[...groups.entries()].map(([scope, skills]) => <View key={scope}><SectionHeader>{`${scope} · ${skills.length}`}</SectionHeader>{skills.map((skill, index) => <View key={skill.name} style={[styles.skill, index < skills.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}><View style={styles.row}><Text style={[styles.name, { color: tokens.ink }]} numberOfLines={1}>{skill.name}</Text>{skill.tier ? <Badge label={skill.tier} tone="accent" /> : null}</View><Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={3}>{skill.description}</Text><Text style={[type.monoMeta, { color: tokens.ink4 }]}>{skill.resources} linked resource{skill.resources === 1 ? "" : "s"}</Text></View>)}</View>)}</Screen>; }

export default function SkillsScreen() {
  return <DesktopDrillDown><SettingsShell active="skills"><SkillsScreenBody /></SettingsShell></DesktopDrillDown>;
}
const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, skill: { gap: space.space4, paddingVertical: space.space12 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold } });
