import React, { useMemo, useState } from "react";
import { RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useSkills } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { Sparkles } from "lucide-react-native";

type SkillRows = NonNullable<ReturnType<typeof useSkills>["data"]>;

export default function SkillsScreen() { const tokens = useTokens(); const query = useSkills(); const skills = query.data; const [search, setSearch] = useState(""); const groups = useMemo(() => { const result = new Map<string, SkillRows>(); for (const skill of skills ?? []) if (!search.trim() || `${skill.name} ${skill.description} ${skill.scope} ${skill.tier ?? ""}`.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())) result.set(skill.scope, [...(result.get(skill.scope) ?? []), skill]); return result; }, [skills, search]); return <DesktopDrillDown><Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><BackLink /><Text style={[type.title, { color: tokens.ink }]}>Skills</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Reusable Forge methodologies available to every session.</Text><SearchField value={search} onChangeText={setSearch} placeholder="Search skills" accessibilityLabel="Search skills" />{query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load skills. Pull to retry.</Text></Card> : null}{!query.isLoading && groups.size === 0 ? <EmptyState icon={Sparkles} message={search ? "No skills match that search." : "No skills are available on this server."} /> : null}{[...groups.entries()].map(([scope, skills]) => <View key={scope}><SectionHeader>{`${scope} · ${skills.length}`}</SectionHeader>{skills.map((skill) => <Card key={skill.name} style={styles.skill}><View style={styles.row}><Text style={[type.body, styles.name, { color: tokens.ink }]} numberOfLines={1}>{skill.name}</Text>{skill.tier ? <Badge label={skill.tier} tone="accent" /> : null}</View><Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={3}>{skill.description}</Text><Text style={[type.sub, { color: tokens.ink3 }]}>{skill.resources} linked resource{skill.resources === 1 ? "" : "s"}</Text></Card>)}</View>)}</Screen></DesktopDrillDown>; }
const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, skill: { gap: space.space4, marginBottom: space.space8 }, row: { flexDirection: "row", alignItems: "center", gap: space.space8 }, name: { flex: 1 } });
