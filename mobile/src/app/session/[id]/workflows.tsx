// Workflow Library. Lists the session's saved workflows (`useWorkflows`) as de-boxed rows;
// the one the active run names (if any) is highlighted with a heat edge + card. Each workflow
// runs from a free-text args field into `/workflow run <name> [args]`, then returns to the
// session. On medium+ it folds into a two-column list + detail (desktop prototype). A live-run
// pill links to the run view whenever a workflow is active.
import { ArrowLeft, ArrowRight, WifiOff, Workflow as WorkflowIcon } from "lucide-react-native";
import { router } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { Button } from "../../../components/ds/Button";
import { EmptyState } from "../../../components/ds/EmptyState";
import { HeatEdge } from "../../../components/ds/HeatEdge";
import { IconButton } from "../../../components/ds/IconButton";
import { Input } from "../../../components/ds/Input";
import { Screen } from "../../../components/ds/Screen";
import { StatusDot } from "../../../components/ds/StatusDot";
import { useToast } from "../../../components/ds/ToastHost";
import type { WorkflowRow } from "../../../lib/api";
import { useWorkflows } from "../../../lib/queries";
import { useSessionCtx } from "../../../lib/sessionContext";
import { useTokens } from "../../../theme/ThemeProvider";
import { radii, space } from "../../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../../theme/typography";
import { useBreakpoint } from "../../../theme/useBreakpoint";

export default function WorkflowLibraryScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { sessionId, snapshot, send } = useSessionCtx();
  const { isCompact } = useBreakpoint();
  const query = useWorkflows(sessionId);
  const rows = useMemo(() => query.data ?? [], [query.data]);

  const workflow = snapshot?.workflow ?? null;
  const activeName = workflow?.active ? workflow.name : null;

  const [selectedRaw, setSelectedRaw] = useState<string | null>(null);
  const [argsByName, setArgsByName] = useState<Record<string, string>>({});

  const names = useMemo(() => rows.map((r) => r.name), [rows]);
  const selectedName =
    selectedRaw && names.includes(selectedRaw)
      ? selectedRaw
      : activeName && names.includes(activeName)
        ? activeName
        : names[0] ?? null;

  const setArgs = useCallback((name: string, text: string) => setArgsByName((prev) => ({ ...prev, [name]: text })), []);
  const openRun = useCallback(() => router.push(`/session/${sessionId}/workflow` as never), [sessionId]);
  const back = useCallback(() => router.back(), []);

  const run = useCallback(
    (name: string) => {
      const args = (argsByName[name] ?? "").trim();
      const text = `/workflow run ${name}${args ? ` ${args}` : ""}`;
      if (send({ kind: "prompt", text })) router.back();
      else toast.show("not sent — reconnect and try again", { tone: "danger" });
    },
    [argsByName, send, toast],
  );

  const header = (
    <View style={styles.headerBlock}>
      <View style={styles.headerRow}>
        <IconButton icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink2} />} onPress={back} accessibilityLabel="Back to session" />
        <Text style={[typeScale.title, { color: tokens.ink }]}>Workflows</Text>
      </View>
      <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>Saved multi-agent runs. Run with arguments and watch live.</Text>
      {activeName ? <LiveRunPill name={activeName} onPress={openRun} /> : null}
    </View>
  );

  // ---- loading / error / empty -------------------------------------------
  if (query.isLoading) {
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        {header}
        <View style={styles.loading}>
          <ActivityIndicator color={tokens.accent} />
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>loading workflows…</Text>
        </View>
      </Screen>
    );
  }

  if (query.isError) {
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        {header}
        <EmptyState icon={WifiOff} message="couldn't load workflows — check the server connection" />
      </Screen>
    );
  }

  if (rows.length === 0) {
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        {header}
        <EmptyState icon={WorkflowIcon} message="Define workflows in .forge/workflows/*.js — they appear here automatically." />
      </Screen>
    );
  }

  // ---- compact: stacked cards --------------------------------------------
  if (isCompact) {
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        {header}
        {rows.map((wf, index) => {
          const isActive = wf.name === activeName;
          const expanded = wf.name === selectedName;
          if (expanded) {
            return (
              <View key={wf.name} style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
                {isActive ? <HeatEdge state="busy" /> : null}
                <View style={styles.cardBody}>
                  <WorkflowDetail
                    workflow={wf}
                    active={isActive}
                    args={argsByName[wf.name] ?? ""}
                    onArgs={(t) => setArgs(wf.name, t)}
                    onRun={() => run(wf.name)}
                    onViewRun={openRun}
                  />
                </View>
              </View>
            );
          }
          return (
            <CollapsedRow
              key={wf.name}
              workflow={wf}
              active={isActive}
              selected={false}
              showSeparator={index < rows.length - 1}
              onPress={() => setSelectedRaw(wf.name)}
            />
          );
        })}
      </Screen>
    );
  }

  // ---- medium+: two-column list + detail ---------------------------------
  const selectedWorkflow = rows.find((r) => r.name === selectedName) ?? null;
  const selectedIsActive = selectedWorkflow?.name === activeName;

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.split}>
      <ScrollView style={styles.listPane} contentContainerStyle={styles.listContent}>
        {header}
        {rows.map((wf, index) => (
          <CollapsedRow
            key={wf.name}
            workflow={wf}
            active={wf.name === activeName}
            selected={wf.name === selectedName}
            showSeparator={index < rows.length - 1}
            onPress={() => setSelectedRaw(wf.name)}
          />
        ))}
      </ScrollView>
      <View style={[styles.detailPane, { borderLeftColor: tokens.border }]}>
        <ScrollView contentContainerStyle={styles.detailContent}>
          {selectedWorkflow ? (
            <WorkflowDetail
              workflow={selectedWorkflow}
              active={selectedIsActive}
              args={argsByName[selectedWorkflow.name] ?? ""}
              onArgs={(t) => setArgs(selectedWorkflow.name, t)}
              onRun={() => run(selectedWorkflow.name)}
              onViewRun={openRun}
            />
          ) : null}
        </ScrollView>
      </View>
    </Screen>
  );
}

function LiveRunPill({ name, onPress }: { name: string; onPress: () => void }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`View live run of ${name ?? "workflow"}`}
      style={[styles.pill, { backgroundColor: tokens.selection, borderColor: tokens.accent }]}
    >
      <StatusDot state="busy" />
      <Text style={[typeScale.monoMeta, styles.pillText, { color: tokens.ink2 }]} numberOfLines={1}>
        {`${name ?? "workflow"} running`}
      </Text>
      <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>view run</Text>
      <ArrowRight size={13} strokeWidth={2} color={tokens.accent} />
    </Pressable>
  );
}

function phasesBreadcrumb(phases: string[]): string {
  return phases.length > 0 ? phases.join("  →  ") : "no phases declared";
}

function CollapsedRow({
  workflow,
  active,
  selected,
  showSeparator,
  onPress,
}: {
  workflow: WorkflowRow;
  active: boolean;
  selected: boolean;
  showSeparator: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.rowWrap, selected ? { backgroundColor: tokens.selection, borderRadius: radii.radius12 } : null]}>
      {active ? <HeatEdge state="busy" /> : null}
      <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={`Workflow ${workflow.name}`} style={styles.row}>
        <View style={styles.rowHeader}>
          <Text style={[styles.name, { color: active ? tokens.accent : tokens.ink }]} numberOfLines={1}>
            {workflow.name}
          </Text>
          {active ? <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>running</Text> : null}
        </View>
        <Text style={[typeScale.sub, styles.desc, { color: tokens.ink2 }]} numberOfLines={2}>
          {workflow.description}
        </Text>
        {workflow.when_to_use ? (
          <Text style={[typeScale.meta, styles.when, { color: tokens.ink4 }]} numberOfLines={1}>
            {`when to use: ${workflow.when_to_use}`}
          </Text>
        ) : null}
        <Text style={[typeScale.monoMeta, styles.breadcrumb, { color: tokens.ink3 }]} numberOfLines={1}>
          {phasesBreadcrumb(workflow.phases)}
        </Text>
      </Pressable>
      {showSeparator && !selected ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

function WorkflowDetail({
  workflow,
  active,
  args,
  onArgs,
  onRun,
  onViewRun,
}: {
  workflow: WorkflowRow;
  active: boolean;
  args: string;
  onArgs: (text: string) => void;
  onRun: () => void;
  onViewRun: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={styles.detail}>
      <View style={styles.detailHeader}>
        {active ? <StatusDot state="busy" /> : null}
        <Text style={[styles.detailName, { color: tokens.ink }]} numberOfLines={1}>
          {workflow.name}
        </Text>
        {active ? <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>running</Text> : null}
      </View>
      <Text style={[typeScale.body, styles.detailDesc, { color: tokens.ink2 }]}>{workflow.description}</Text>
      {workflow.when_to_use ? (
        <Text style={[typeScale.meta, { color: tokens.ink4 }]}>{`when to use: ${workflow.when_to_use}`}</Text>
      ) : null}

      <Text style={[typeScale.section, styles.detailLabel, { color: tokens.ink4 }]}>phases</Text>
      <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{phasesBreadcrumb(workflow.phases)}</Text>

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
      <Button label="Run workflow" onPress={onRun} fullWidth />

      {active ? (
        <Pressable onPress={onViewRun} accessibilityRole="button" accessibilityLabel="View live run" style={styles.viewRun}>
          <Text style={[typeScale.bodyBold, { color: tokens.accent }]}>View live run</Text>
          <ArrowRight size={15} strokeWidth={2} color={tokens.accent} />
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  column: { width: "100%", maxWidth: 760, alignSelf: "center", paddingTop: space.space8, paddingBottom: space.space32 },
  headerBlock: { gap: space.space8, marginBottom: space.space8 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space4, marginLeft: -space.space8 },
  subtitle: { marginTop: -space.space4 },
  loading: { alignItems: "center", justifyContent: "center", padding: space.space32, gap: space.space12 },

  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    alignSelf: "flex-start",
    minHeight: 36,
    paddingHorizontal: space.space12,
    borderRadius: radii.radiusPill,
    borderWidth: StyleSheet.hairlineWidth,
    marginTop: space.space4,
  },
  pillText: { flexShrink: 1 },

  card: {
    position: "relative",
    marginTop: space.space16,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius16,
    overflow: "hidden",
  },
  cardBody: { padding: space.space16 },

  rowWrap: { position: "relative", marginTop: space.space4 },
  row: { paddingLeft: space.space8, paddingRight: space.space8, paddingVertical: space.space12, gap: space.space4 },
  rowHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1, fontFamily: monoFamily.bold, fontSize: 14, fontWeight: "700" },
  desc: {},
  when: {},
  breadcrumb: { marginTop: space.space2 },
  separator: { height: StyleSheet.hairlineWidth, marginTop: space.space12, marginHorizontal: space.space8 },

  split: { flex: 1, flexDirection: "row" },
  listPane: { flex: 0.42, minWidth: 0 },
  listContent: { paddingTop: space.space8, paddingBottom: space.space32, paddingRight: space.space24 },
  detailPane: { flex: 0.58, minWidth: 0, borderLeftWidth: StyleSheet.hairlineWidth },
  detailContent: { paddingTop: space.space16, paddingBottom: space.space32, paddingLeft: space.space24 },

  detail: { gap: space.space8 },
  detailHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  detailName: { flex: 1, fontFamily: monoFamily.bold, fontSize: 16, fontWeight: "700" },
  detailDesc: {},
  detailLabel: { marginTop: space.space8 },
  argsInput: { marginTop: space.space8 },
  viewRun: { flexDirection: "row", alignItems: "center", gap: space.space4, minHeight: 44, marginTop: space.space4 },
});
