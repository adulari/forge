// Skills (Native Features pack — "NF Skills" / "NF Desktop Skills"). Source-badged
// catalog rows: mono skill name + source badge (builtin/user/project from scope) +
// copyable mono /slash-name + description, with an expandable body (compact) or a
// folded detail pane (desktop).
//
// This screen lives in the settings family (entered via /skills from Settings — no
// session is reachable from here), so the prototype's in-context "Run" affordance would
// have nothing to run against. Per the task, the mono /slash-name is offered as a COPY
// action instead (tap → clipboard + toast), so a session can paste it later.
//
// Wire scope (lib/api.ts SkillRow = {name, description, scope, tier, resources}): the
// prototype's rendered guidance markdown and the auto-vs-slash distinction are not on the
// contract, so the detail shows description + resources + scope + tier only, with a note
// that full guidance is applied in-session (see report).
import * as Clipboard from "expo-clipboard";
import { ChevronDown, Sparkles } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View, type ViewStyle } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useToast } from "../components/ds/ToastHost";
import { type SkillRow } from "../lib/api";
import { useSkills } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { type ColorTokens, radii, space, tapTarget } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";
import { SettingsShell } from "./(tabs)/settings";

type Scope = SkillRow["scope"];
const SCOPE_ORDER: Scope[] = ["project", "builtin", "user"];

// Derive a translucent surface from a token hex — the PROJECT source badge is info-toned
// and there is no `infoBg` token (only successBg/dangerBg/warnBg), so it is tinted here
// from tokens.info exactly as the prototype's rgba(79,208,217,.12) chip. Token-derived,
// never a raw literal.
function tint(hex: string, alpha: number): string {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function scopeColors(scope: Scope, tokens: ColorTokens): { bg: string; ink: string } {
  switch (scope) {
    case "builtin":
      return { bg: tokens.selection, ink: tokens.accent };
    case "project":
      return { bg: tint(tokens.info, 0.12), ink: tokens.info };
    case "user":
    default:
      return { bg: tokens.bg3, ink: tokens.ink2 };
  }
}

function SourceBadge({ scope }: { scope: Scope }) {
  const tokens = useTokens();
  const { bg, ink } = scopeColors(scope, tokens);
  return (
    <View style={[styles.sourceBadge, { backgroundColor: bg }]} accessibilityRole="text" accessibilityLabel={`${scope} skill`}>
      <Text style={[styles.sourceBadgeText, { color: ink }]}>{scope.toUpperCase()}</Text>
    </View>
  );
}

function SlashPill({ name }: { name: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const slash = `/${name}`;
  return (
    <Pressable
      onPress={() => {
        void Clipboard.setStringAsync(slash).then(() => toast.show(`Copied ${slash}`));
      }}
      accessibilityRole="button"
      accessibilityLabel={`Copy ${slash}`}
      style={[styles.slashPill, { backgroundColor: tokens.bg3 }]}
    >
      <Text style={[styles.slashText, { color: tokens.accent }]} numberOfLines={1}>
        {slash}
      </Text>
    </Pressable>
  );
}

function ResourcesMeta({ resources }: { resources: number }) {
  const tokens = useTokens();
  return (
    <Text style={[type.monoMeta, { color: tokens.ink4 }]}>
      {`${resources} linked resource${resources === 1 ? "" : "s"}`}
    </Text>
  );
}

function SkillRowItem({ skill, expanded, onToggle, showSeparator }: { skill: SkillRow; expanded: boolean; onToggle: () => void; showSeparator: boolean }) {
  const tokens = useTokens();
  const cardStyle: ViewStyle = expanded
    ? { backgroundColor: tokens.bg2, borderColor: tokens.border, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, paddingHorizontal: space.space16 }
    : {};
  const separator = showSeparator && !expanded ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : undefined;
  return (
    <View style={[styles.skill, cardStyle, separator]}>
      <View style={styles.topline}>
        <Text style={[styles.name, { color: tokens.ink }]} numberOfLines={1}>
          {skill.name}
        </Text>
        <SourceBadge scope={skill.scope} />
        <SlashPill name={skill.name} />
        <Pressable
          onPress={onToggle}
          accessibilityRole="button"
          accessibilityLabel={expanded ? `Collapse ${skill.name}` : `Expand ${skill.name}`}
          accessibilityState={{ expanded }}
          style={styles.chevron}
        >
          <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink3} style={expanded ? styles.chevronOpen : undefined} />
        </Pressable>
      </View>
      <Text style={[type.sub, { color: tokens.ink2 }]} numberOfLines={expanded ? undefined : 2}>
        {skill.description}
      </Text>
      {expanded ? (
        <View style={styles.expandBody}>
          {skill.tier ? (
            <View style={[styles.tierChip, { backgroundColor: tokens.bg3 }]}>
              <Text style={[type.meta, { color: tokens.ink2 }]}>{skill.tier}</Text>
            </View>
          ) : null}
          <ResourcesMeta resources={skill.resources} />
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>Full guidance is applied in-session — not synced here.</Text>
        </View>
      ) : (
        <ResourcesMeta resources={skill.resources} />
      )}
    </View>
  );
}

function DetailPane({ skill }: { skill: SkillRow | null }) {
  const tokens = useTokens();
  if (!skill) {
    return (
      <View style={[styles.detailCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
        <Text style={[type.sub, { color: tokens.ink3 }]}>Select a skill to see its details.</Text>
      </View>
    );
  }
  return (
    <View style={[styles.detailCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={styles.detailHead}>
        <Text style={[styles.detailName, { color: tokens.ink }]} numberOfLines={1}>
          {skill.name}
        </Text>
        <SourceBadge scope={skill.scope} />
        <SlashPill name={skill.name} />
      </View>
      <Text style={[type.sub, { color: tokens.ink2 }]}>{skill.description}</Text>
      <Text style={[type.section, styles.detailSection, { color: tokens.ink4 }]}>Details</Text>
      {skill.tier ? (
        <View style={[styles.tierChip, { backgroundColor: tokens.bg3 }]}>
          <Text style={[type.meta, { color: tokens.ink2 }]}>{skill.tier}</Text>
        </View>
      ) : null}
      <ResourcesMeta resources={skill.resources} />
      <Text style={[type.monoMeta, { color: tokens.ink4 }]}>Full guidance is applied in-session — not synced to this view.</Text>
    </View>
  );
}

function SkillsScreenBody() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const query = useSkills();
  const [search, setSearch] = useState("");
  const [activeName, setActiveName] = useState<string | null>(null);
  const needle = search.trim().toLocaleLowerCase();

  const filtered = useMemo(
    () => (query.data ?? []).filter((skill) => !needle || `${skill.name} ${skill.description} ${skill.scope} ${skill.tier ?? ""}`.toLocaleLowerCase().includes(needle)),
    [query.data, needle],
  );

  const groups = useMemo(() => {
    const byScope = new Map<Scope, SkillRow[]>();
    for (const skill of filtered) byScope.set(skill.scope, [...(byScope.get(skill.scope) ?? []), skill]);
    return SCOPE_ORDER.filter((scope) => byScope.has(scope)).map((scope) => [scope, byScope.get(scope) ?? []] as const);
  }, [filtered]);

  // Desktop detail pane always shows a valid selection: the active skill if it survives
  // the current filter, else the first filtered skill.
  const selected = useMemo(
    () => filtered.find((skill) => skill.name === activeName) ?? filtered[0] ?? null,
    [filtered, activeName],
  );

  const onRowToggle = (name: string) => {
    if (isExpanded) setActiveName(name);
    else setActiveName((current) => (current === name ? null : name));
  };

  const listContent =
    query.isError && !query.data ? (
      <Text style={[type.body, { color: tokens.danger }]}>Could not load skills. Pull to retry.</Text>
    ) : !query.isLoading && filtered.length === 0 ? (
      <EmptyState icon={Sparkles} message={search ? "No skills match that search." : "No skills are available on this server."} />
    ) : (
      <View>
        {groups.map(([scope, rows]) => (
          <View key={scope}>
            <SectionHeader>{`${scope} · ${rows.length}`}</SectionHeader>
            {rows.map((skill, index) => (
              <SkillRowItem
                key={skill.name}
                skill={skill}
                expanded={isExpanded ? selected?.name === skill.name : activeName === skill.name}
                onToggle={() => onRowToggle(skill.name)}
                showSeparator={index < rows.length - 1}
              />
            ))}
          </View>
        ))}
      </View>
    );

  return (
    <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Skills</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>Reusable Forge methodologies available to every session.</Text>
      <SearchField value={search} onChangeText={setSearch} placeholder="Search skills" accessibilityLabel="Search skills" />
      {isExpanded && filtered.length > 0 ? (
        <View style={styles.twoCol}>
          <View style={styles.colMain}>{listContent}</View>
          <View style={styles.colSide}>
            <DetailPane skill={selected} />
          </View>
        </View>
      ) : (
        listContent
      )}
    </Screen>
  );
}

export default function SkillsScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="skills">
        <SkillsScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  twoCol: { flexDirection: "row", gap: space.space32, alignItems: "flex-start" },
  colMain: { flex: 1, minWidth: 0 },
  colSide: { flex: 1, minWidth: 0 },
  skill: { gap: space.space4, paddingVertical: space.space12 },
  topline: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold },
  sourceBadge: { borderRadius: radii.radiusPill, paddingHorizontal: space.space8, paddingVertical: 2 },
  sourceBadgeText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.3 },
  slashPill: { minHeight: 26, justifyContent: "center", borderRadius: radii.radiusPill, paddingHorizontal: 11 },
  slashText: { fontSize: 11, fontFamily: monoFamily.regular },
  chevron: { width: tapTarget, height: tapTarget, alignItems: "center", justifyContent: "center", marginVertical: -space.space8, marginRight: -space.space8 },
  chevronOpen: { transform: [{ rotate: "180deg" }] },
  expandBody: { gap: space.space4, marginTop: space.space4 },
  tierChip: { alignSelf: "flex-start", borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: 2 },
  detailCard: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space16, gap: space.space8 },
  detailHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  detailName: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold },
  detailSection: { marginTop: space.space4 },
});
