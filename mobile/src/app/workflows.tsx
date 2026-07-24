// Machined Workflows library (top-level route — "M/D Workflow Library" frames): definition
// cards (name, description, phase chips, free-text args, Run button) for every workflow
// `.forge/workflows/` exposes. Reachable from Settings, not scoped to a live session, so
// there is no active-run pill or run-history strip here (the wire has neither a run-history
// endpoint nor a typed-args schema on `WorkflowRow` — see file-level honesty note below).
// Running a workflow hands its `/workflow run <name> [args]` command to a brand-new session
// via the same `?title=` handoff the Fleet composer uses (`new-session.tsx` reads it back
// as the session's first prompt) — this route only browses definitions, it never talks to a
// live session directly.
import { router } from "expo-router";
import { ArrowRight, Workflow as WorkflowIcon } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { ActivityIndicator, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { Button } from "../components/ds/Button";
import { EmptyState } from "../components/ds/EmptyState";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import type { WorkflowRow } from "../lib/api";
import { useWorkflows } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

function phasesBreadcrumb(phases: string[]): string {
  return phases.length > 0 ? phases.join("  →  ") : "no phases declared";
}

function WorkflowCard({
  workflow,
  args,
  onArgs,
  onRun,
}: {
  workflow: WorkflowRow;
  args: string;
  onArgs: (text: string) => void;
  onRun: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={styles.cardHead}>
        <Text style={[styles.name, { color: tokens.ink }]} numberOfLines={1}>
          {workflow.name}
        </Text>
      </View>
      <Text style={[typeScale.sub, styles.desc, { color: tokens.ink2 }]}>{workflow.description}</Text>
      {workflow.when_to_use ? (
        <Text style={[typeScale.meta, { color: tokens.ink4 }]}>{`when to use: ${workflow.when_to_use}`}</Text>
      ) : null}

      {workflow.phases.length > 0 ? (
        <View style={styles.chips}>
          {workflow.phases.map((phase) => (
            <View key={phase} style={[styles.chip, { backgroundColor: tokens.bg3 }]}>
              <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>{phase}</Text>
            </View>
          ))}
        </View>
      ) : (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>{phasesBreadcrumb(workflow.phases)}</Text>
      )}

      <Input
        label="arguments (optional)"
        value={args}
        onChangeText={onArgs}
        placeholder="version=2.7.0 dry_run=false"
        mono
        autoCapitalize="none"
        autoCorrect={false}
        numberOfLines={1}
        onSubmitEditing={onRun}
        containerStyle={styles.argsInput}
      />
      <Button label="Run workflow" onPress={onRun} fullWidth style={styles.runBtn} icon={<ArrowRight size={15} strokeWidth={2} color={tokens.bg2} />} />
    </View>
  );
}

function WorkflowsScreenBody() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const query = useWorkflows();
  const rows = query.data ?? [];
  const [argsByName, setArgsByName] = useState<Record<string, string>>({});

  const setArgs = useCallback((name: string, text: string) => setArgsByName((prev) => ({ ...prev, [name]: text })), []);

  const run = useCallback(
    (workflow: WorkflowRow) => {
      const args = (argsByName[workflow.name] ?? "").trim();
      const text = `/workflow run ${workflow.name}${args ? ` ${args}` : ""}`;
      router.push({ pathname: "/new-session", params: { title: text } });
    },
    [argsByName],
  );

  return (
    <Screen
      scroll
      refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />}
      contentContainerStyle={styles.content}
    >
      <BackLink />
      <View style={styles.headerRow}>
        <Text style={[typeScale.title, { color: tokens.ink }]}>Workflows</Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>.forge/workflows/</Text>
      </View>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
        Saved multi-agent runs. Running one starts a new session with the run as its first prompt.
      </Text>

      {query.isLoading ? (
        <View style={styles.loading}>
          <ActivityIndicator color={tokens.accent} />
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>loading workflows…</Text>
        </View>
      ) : query.isError ? (
        <EmptyState icon={WorkflowIcon} message="Could not load workflows — check the server connection." />
      ) : rows.length === 0 ? (
        <EmptyState icon={WorkflowIcon} message="Define workflows in .forge/workflows/*.js — they appear here automatically." />
      ) : (
        <View style={[styles.grid, isExpanded ? styles.gridWide : null]}>
          {rows.map((workflow) => (
            <View key={workflow.name} style={isExpanded ? styles.gridCell : undefined}>
              <WorkflowCard
                workflow={workflow}
                args={argsByName[workflow.name] ?? ""}
                onArgs={(text) => setArgs(workflow.name, text)}
                onRun={() => run(workflow)}
              />
            </View>
          ))}
        </View>
      )}
    </Screen>
  );
}

export default function WorkflowsScreen() {
  return (
    <DesktopDrillDown>
      <WorkflowsScreenBody />
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space8, width: "100%", maxWidth: 1000, alignSelf: "center" },
  headerRow: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  loading: { alignItems: "center", justifyContent: "center", padding: space.space32, gap: space.space12 },
  grid: { marginTop: space.space12, gap: space.space16 },
  gridWide: { flexDirection: "row", flexWrap: "wrap" },
  gridCell: { width: "48%" },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space16, gap: space.space8 },
  cardHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1, fontFamily: monoFamily.bold, fontSize: 15 },
  desc: {},
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, marginTop: space.space4 },
  chip: { borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: 2 },
  argsInput: { marginTop: space.space8 },
  runBtn: { marginTop: space.space4 },
});
