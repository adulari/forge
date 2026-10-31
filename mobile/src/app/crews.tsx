// Saved crews (Settings > Crews): named rosters of subagent roles. `@crew:<name>` in a prompt
// (or `/crew <name>` in the TUI) makes the model delegate to the roster. Rows show the member
// count and role chips; a row opens the editor sheet (read-only for project crews).
import { Plus, Users } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { CrewSheet } from "../components/crews/CrewSheet";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Button } from "../components/ds/Button";
import { EmptyState } from "../components/ds/EmptyState";
import { PageHeader } from "../components/ds/PageHeader";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { Skeleton } from "../components/ds/Skeleton";
import type { Crew } from "../lib/crews";
import { useCrews } from "../lib/queries";
import { usePullRefresh } from "../lib/usePullRefresh";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

function RoleChip({ role }: { role: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.chip, { backgroundColor: tokens.bg3 }]}>
      <Text style={[styles.chipText, { color: tokens.ink2 }]} numberOfLines={1}>
        {role}
      </Text>
    </View>
  );
}

export function CrewRowItem({ crew, onPress, showSeparator }: { crew: Crew; onPress: () => void; showSeparator: boolean }) {
  const tokens = useTokens();
  const count = `${crew.members.length} ${crew.members.length === 1 ? "member" : "members"}`;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${crew.name}, ${count}${crew.editable ? "" : ", project crew"}`}
      style={[styles.row, showSeparator ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}
    >
      <View style={styles.rowTop}>
        <Text style={[styles.name, { color: tokens.ink }]} numberOfLines={1}>
          {crew.name}
        </Text>
        {crew.editable ? null : (
          <View style={[styles.tag, { borderColor: tokens.border }]}>
            <Text style={[styles.tagText, { color: tokens.ink3 }]}>project crew</Text>
          </View>
        )}
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{count}</Text>
      </View>
      {crew.description ? (
        <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={2}>
          {crew.description}
        </Text>
      ) : null}
      <View style={styles.chips}>
        {crew.members.map((m) => (
          <RoleChip key={m.role} role={m.role} />
        ))}
      </View>
    </Pressable>
  );
}

function CrewRowSkeleton() {
  return (
    <View style={styles.row}>
      <Skeleton width="40%" height={14} />
      <Skeleton width="70%" height={12} />
    </View>
  );
}

function CrewsScreenBody() {
  const tokens = useTokens();
  const query = useCrews();
  const pull = usePullRefresh(query.refetch);
  const [search, setSearch] = useState("");
  const [sheet, setSheet] = useState<{ crew: Crew | null } | null>(null);
  const needle = search.trim().toLocaleLowerCase();
  const crews = useMemo(
    () => (query.data ?? []).filter((c) => !needle || `${c.name} ${c.description} ${c.members.map((m) => m.role).join(" ")}`.toLocaleLowerCase().includes(needle)),
    [query.data, needle],
  );

  const newButton = (
    <Button label="New crew" variant="secondary" icon={<Plus size={16} strokeWidth={1.75} color={tokens.ink} />} onPress={() => setSheet({ crew: null })} accessibilityLabel="Create a new crew" />
  );

  const list = query.isLoading ? (
    <View>
      {[0, 1, 2].map((i) => (
        <CrewRowSkeleton key={i} />
      ))}
    </View>
  ) : query.isError && !query.data ? (
    <EmptyState icon={Users} message="Could not load crews." action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} accessibilityLabel="Retry loading crews" />} />
  ) : crews.length === 0 ? (
    <EmptyState
      icon={Users}
      message={
        search
          ? "No crews match that search."
          : "No crews yet. A crew is a named roster of subagents. Save one, then write @crew:name in a prompt and the model delegates to it."
      }
      action={search ? undefined : newButton}
    />
  ) : (
    <View>
      {crews.map((crew, index) => (
        <CrewRowItem key={crew.name} crew={crew} onPress={() => setSheet({ crew })} showSeparator={index < crews.length - 1} />
      ))}
    </View>
  );

  return (
    <Screen scroll wide refreshControl={<RefreshControl refreshing={pull.refreshing} onRefresh={pull.onRefresh} />} contentContainerStyle={styles.content}>
      <PageHeader title="Crews" subtitle="Saved rosters of subagents. Mention one as @crew:name in a prompt." trailing={newButton} />
      <SearchField value={search} onChangeText={setSearch} placeholder="Search crews" accessibilityLabel="Search crews" />
      {list}
      <CrewSheet visible={sheet != null} crew={sheet?.crew ?? null} onClose={() => setSheet(null)} />
    </Screen>
  );
}

export default function CrewsScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="crews">
        <CrewsScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  row: { paddingVertical: space.space12, gap: space.space4, minHeight: 44 },
  rowTop: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1, fontSize: 14, fontFamily: monoFamily.bold },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space4, marginTop: space.space4 },
  chip: { borderRadius: radii.radius4, paddingHorizontal: 7, paddingVertical: 2, maxWidth: "100%" },
  chipText: { fontSize: 11, fontFamily: monoFamily.regular },
  tag: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radiusPill, paddingHorizontal: space.space8, paddingVertical: 1 },
  tagText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.3 },
});
