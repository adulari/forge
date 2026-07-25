// Machined Workflows library ("D Workflow Library" L649-666 / "M Workflows" L228-253): definition
// cards (name, description, phase chips, typed argument fields, Run button, run-history strip) for
// every workflow `.forge/workflows/` exposes. Reachable from Settings, not scoped to a live
// session, so there is no active-run pill here.
//
// Running a workflow hands its `/workflow run <name> [args]` command to a brand-new session via
// the same `?title=` handoff the Fleet composer uses (`new-session.tsx` reads it back as the
// session's first prompt) — this route only browses definitions, it never talks to a live session
// directly. `/workflow run` passes everything after the name to the script verbatim as a single
// string (see `dispatch.rs` WorkflowAction::RunSaved), so declared args are composed back into
// `name=value` words rather than sent as structured JSON.
//
// HONESTY: `WorkflowRow.args` is empty for every workflow that doesn't declare `meta.args` (most
// don't), and `runs` is empty for a workflow that has never run on this machine. Both render
// explicit empty states; neither is ever filled with reconstructed rows.
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
import type { WorkflowArg, WorkflowRow, WorkflowRun } from "../lib/api";
import { useWorkflows } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { formatCost, formatRelativeTime, monoFamily, tabularNums, type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

type ArgValues = Record<string, string>;

function phasesBreadcrumb(phases: string[]): string {
  return phases.length > 0 ? phases.join("  →  ") : "no phases declared";
}

/** The declared default is the starting value, so an untouched field still runs what the author
 * intended; clearing it deliberately (empty string, not undefined) keeps it cleared. */
function argValue(arg: WorkflowArg, values: ArgValues): string {
  return values[arg.name] ?? arg.default ?? "";
}

function argCaption(arg: WorkflowArg): string {
  return [arg.arg_type, arg.required ? "required" : "optional", arg.default ? `default ${arg.default}` : null]
    .filter(Boolean)
    .join(" · ");
}

/** Declared args become `name=value` words; a value with whitespace is quoted so the script sees
 * one token. Untouched optional args with no value are dropped rather than sent empty. */
function composeArgs(workflow: WorkflowRow, values: ArgValues, freeText: string): string {
  if (workflow.args.length === 0) return freeText.trim();
  return workflow.args
    .map((arg) => [arg.name, argValue(arg, values).trim()] as const)
    .filter(([, value]) => value.length > 0)
    .map(([name, value]) => `${name}=${/\s/.test(value) ? JSON.stringify(value) : value}`)
    .join(" ");
}

function missingRequired(workflow: WorkflowRow, values: ArgValues): boolean {
  return workflow.args.some((arg) => arg.required && argValue(arg, values).trim().length === 0);
}

/** The strip's mark + tint per status. `interrupted` is deliberately NOT rendered as a failure:
 * an interrupted run (Esc, a killed process, a crash) never reported an outcome, so calling it
 * failed would be inventing one — it gets the warn tint and says so in words. */
function runTone(status: WorkflowRun["status"], tokens: ReturnType<typeof useTokens>): { mark: string; color: string } {
  if (status === "running") return { mark: "●", color: tokens.accent };
  if (status === "ok") return { mark: "✓", color: tokens.success };
  if (status === "failed") return { mark: "✗", color: tokens.danger };
  return { mark: "!", color: tokens.warn };
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

function RunHistory({ runs }: { runs: WorkflowRun[] }) {
  const tokens = useTokens();
  if (runs.length === 0) {
    return <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>no runs recorded yet</Text>;
  }
  return (
    <View style={styles.runs}>
      {runs.map((run, index) => {
        const { mark, color } = runTone(run.status, tokens);
        // LIVENESS COMES FROM `status`, NOT `finished_at`: a run killed mid-flight has no
        // `finished_at` either, so reading the old `finished_at == null` test would have left it
        // claiming "running" forever.
        const running = run.status === "running";
        const parts = [
          // Epoch seconds, matching every other timestamp on this wire (`created_at`, `last_activity`).
          running ? "running" : formatRelativeTime(run.started_at * 1000),
          run.status === "failed" || run.status === "interrupted" ? run.status : null,
          // The counts are closed out when a run finishes, so a live one still reads 0/0 —
          // showing those would be a claim about work that hasn't been counted yet.
          running ? null : `${plural(run.phases, "phase")} · ${plural(run.agents, "agent")}`,
          // 0 means no agent reported a cost, which is not the same as a run that was free.
          run.cost_usd > 0 ? formatCost(run.cost_usd) : null,
          run.summary,
        ].filter(Boolean);
        return (
          <Text
            key={`${run.started_at}-${index}`}
            style={[typeScale.monoMeta, tabularNums, { color }]}
            numberOfLines={1}
            accessibilityRole="button"
            accessibilityLabel={`open the session of the ${run.status} run`}
            onPress={() => router.push(`/session/${run.session_id}`)}
          >
            {`${mark} ${parts.join(" · ")}`}
          </Text>
        );
      })}
    </View>
  );
}

function WorkflowCard({
  workflow,
  freeText,
  onFreeText,
  values,
  onValue,
  onRun,
}: {
  workflow: WorkflowRow;
  freeText: string;
  onFreeText: (text: string) => void;
  values: ArgValues;
  onValue: (name: string, text: string) => void;
  onRun: () => void;
}) {
  const tokens = useTokens();
  const blocked = missingRequired(workflow, values);
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

      {workflow.args.length > 0 ? (
        <View style={styles.args}>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>args</Text>
          {workflow.args.map((arg) => (
            <View key={arg.name} style={styles.argField}>
              <Input
                label={arg.name}
                value={argValue(arg, values)}
                onChangeText={(text) => onValue(arg.name, text)}
                placeholder={arg.default ?? arg.arg_type ?? "value"}
                mono
                autoCapitalize="none"
                autoCorrect={false}
                numberOfLines={1}
                onSubmitEditing={blocked ? undefined : onRun}
                accessibilityLabel={`${workflow.name} argument ${arg.name}`}
              />
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{argCaption(arg)}</Text>
              {arg.description ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{arg.description}</Text> : null}
            </View>
          ))}
        </View>
      ) : (
        <Input
          label="arguments (optional)"
          value={freeText}
          onChangeText={onFreeText}
          placeholder="version=2.7.0 dry_run=false"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          numberOfLines={1}
          onSubmitEditing={onRun}
          containerStyle={styles.argsInput}
        />
      )}

      <Button
        label="Run workflow"
        onPress={onRun}
        disabled={blocked}
        fullWidth
        style={styles.runBtn}
        icon={<ArrowRight size={15} strokeWidth={2} color={tokens.bg2} />}
      />
      {blocked ? (
        <Text style={[typeScale.monoMeta, { color: tokens.warnBgInk }]}>fill every required argument to run</Text>
      ) : null}
      <RunHistory runs={workflow.runs} />
    </View>
  );
}

function WorkflowsScreenBody() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const query = useWorkflows();
  const rows = query.data ?? [];
  const [freeTextByName, setFreeTextByName] = useState<Record<string, string>>({});
  const [valuesByName, setValuesByName] = useState<Record<string, ArgValues>>({});

  const setFreeText = useCallback(
    (name: string, text: string) => setFreeTextByName((prev) => ({ ...prev, [name]: text })),
    [],
  );
  const setValue = useCallback(
    (workflow: string, arg: string, text: string) =>
      setValuesByName((prev) => ({ ...prev, [workflow]: { ...(prev[workflow] ?? {}), [arg]: text } })),
    [],
  );

  const run = useCallback(
    (workflow: WorkflowRow) => {
      const args = composeArgs(workflow, valuesByName[workflow.name] ?? {}, freeTextByName[workflow.name] ?? "");
      const text = `/workflow run ${workflow.name}${args ? ` ${args}` : ""}`;
      router.push({ pathname: "/new-session", params: { title: text } });
    },
    [freeTextByName, valuesByName],
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
                freeText={freeTextByName[workflow.name] ?? ""}
                onFreeText={(text) => setFreeText(workflow.name, text)}
                values={valuesByName[workflow.name] ?? {}}
                onValue={(arg, text) => setValue(workflow.name, arg, text)}
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
  args: { marginTop: space.space8, gap: space.space8 },
  argField: { gap: space.space2 },
  argsInput: { marginTop: space.space8 },
  runBtn: { marginTop: space.space4 },
  runs: { gap: space.space2, marginTop: space.space4 },
});
