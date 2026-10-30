import AsyncStorage from "@react-native-async-storage/async-storage";
import { ChevronDown, ChevronRight, ExternalLink, Pin, Play, RotateCcw, Square, Trash2, Workflow } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { DispatchInput, WorkflowRow, WorkflowRunRow } from "../../lib/apiPr";
import {
  useDispatchInputs,
  useDispatchWorkflow,
  usePrAvailable,
  useRunAction,
  useRunDetail,
  useWorkflowRuns,
  useWorkflows,
} from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatRelativeTime, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";
import { isRunActive, orderWorkflows, parsePinned, runLabel, runTone, togglePinned } from "../pr/prModel";
import { toneColor } from "../pr/PrChip";

const PINNED_KEY = "forge.actions.pinned";

const ago = (iso: string): string => (Number.isNaN(Date.parse(iso)) ? "" : formatRelativeTime(Date.parse(iso)));

function usePinned(): [string[], (path: string) => void] {
  const [pinned, setPinned] = useState<string[]>([]);
  useEffect(() => {
    let live = true;
    AsyncStorage.getItem(PINNED_KEY)
      .then((stored) => {
        if (live) setPinned(parsePinned(stored));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  const toggle = useCallback((path: string) => {
    setPinned((current) => {
      const next = togglePinned(current, path);
      AsyncStorage.setItem(PINNED_KEY, JSON.stringify(next)).catch(() => undefined);
      return next;
    });
  }, []);
  return [pinned, toggle];
}

function DispatchSheet({
  sessionId,
  workflow,
  onClose,
}: {
  sessionId: string;
  workflow: WorkflowRow | null;
  onClose: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const inputs = useDispatchInputs(sessionId, workflow?.path ?? null);
  const dispatch = useDispatchWorkflow();
  const [ref, setRef] = useState("");
  const [values, setValues] = useState<Record<string, string | boolean>>({});
  useEffect(() => {
    setRef("");
    setValues({});
  }, [workflow?.path]);

  const defs: DispatchInput[] = inputs.data?.inputs ?? [];
  const missing = defs.some((d) => d.required && (values[d.name] ?? d.default ?? "") === "");
  const run = () => {
    if (!workflow) return;
    const payload: Record<string, string | boolean> = {};
    for (const def of defs) {
      const value = values[def.name] ?? (def.kind === "boolean" ? def.default === "true" : def.default);
      if (value !== undefined && value !== null && value !== "") payload[def.name] = value;
    }
    dispatch.mutate(
      { session: sessionId, workflow: workflow.path, ref: ref.trim() || undefined, inputs: payload },
      {
        onSuccess: () => {
          toast.show(`Dispatched ${workflow.name}`, { tone: "success" });
          onClose();
        },
        onError: (e) => toast.show(e.message, { tone: "danger" }),
      },
    );
  };

  return (
    <Sheet visible={workflow != null} onClose={onClose} accessibilityLabel="Run workflow" snapPoints={[0.8]}>
      <ScrollView contentContainerStyle={styles.sheet} keyboardShouldPersistTaps="handled">
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Run {workflow?.name}</Text>
        <Input label="Branch or tag (optional)" value={ref} onChangeText={setRef} placeholder="default branch" autoCapitalize="none" mono accessibilityLabel="Ref to run on" />
        {inputs.isPending ? <ActivityIndicator color={tokens.accent} /> : null}
        {defs.map((def) =>
          def.kind === "boolean" ? (
            <View key={def.name} style={styles.switchRow}>
              <View style={styles.flex}>
                <Text style={[typeScale.body, { color: tokens.ink }]}>{def.name}</Text>
                {def.description ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{def.description}</Text> : null}
              </View>
              <Switch
                value={(values[def.name] as boolean | undefined) ?? def.default === "true"}
                onValueChange={(v) => setValues((c) => ({ ...c, [def.name]: v }))}
                accessibilityLabel={def.name}
              />
            </View>
          ) : def.options.length > 0 ? (
            <View key={def.name} style={styles.optionBlock}>
              <Text style={[typeScale.meta, { color: tokens.ink2 }]}>
                {def.name}
                {def.required ? " *" : ""}
              </Text>
              <View style={styles.chips}>
                {def.options.map((option) => (
                  <Chip
                    key={option}
                    label={option}
                    size="compact"
                    selected={(values[def.name] ?? def.default) === option}
                    onPress={() => setValues((c) => ({ ...c, [def.name]: option }))}
                  />
                ))}
              </View>
            </View>
          ) : (
            <Input
              key={def.name}
              label={`${def.name}${def.required ? " *" : ""}`}
              value={String(values[def.name] ?? def.default ?? "")}
              onChangeText={(v) => setValues((c) => ({ ...c, [def.name]: v }))}
              placeholder={def.description}
              autoCapitalize="none"
              accessibilityLabel={def.name}
            />
          ),
        )}
        <Button label="Run workflow" onPress={run} disabled={missing} loading={dispatch.isPending} fullWidth />
      </ScrollView>
    </Sheet>
  );
}

function RunJobs({ sessionId, runId }: { sessionId: string; runId: number }) {
  const tokens = useTokens();
  const detail = useRunDetail(sessionId, runId);
  if (detail.isPending) return <ActivityIndicator color={tokens.accent} style={styles.jobs} />;
  if (detail.error) return <Text style={[typeScale.sub, styles.jobs, { color: tokens.danger }]}>{detail.error.message}</Text>;
  return (
    <View style={styles.jobs}>
      {detail.data?.jobs.map((job) => (
        <View key={job.name} style={styles.job}>
          <View style={styles.row}>
            <View style={[styles.dot, { backgroundColor: toneColor(runTone(job), tokens) }]} />
            <Text style={[typeScale.meta, styles.flex, { color: tokens.ink }]} numberOfLines={1}>
              {job.name}
            </Text>
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{runLabel(job)}</Text>
          </View>
          {job.steps.map((step) => (
            <View key={step.number} style={[styles.row, styles.step]}>
              <View style={[styles.dotSmall, { backgroundColor: toneColor(runTone(step), tokens) }]} />
              <Text style={[typeScale.monoMeta, styles.flex, { color: tokens.ink2 }]} numberOfLines={1}>
                {step.name}
              </Text>
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{runLabel(step)}</Text>
            </View>
          ))}
        </View>
      ))}
    </View>
  );
}

function RunRow({
  sessionId,
  run,
  onDelete,
}: {
  sessionId: string;
  run: WorkflowRunRow;
  onDelete: (run: WorkflowRunRow) => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const rerun = useRunAction("rerun");
  const cancel = useRunAction("cancel");
  const active = isRunActive(run);
  const failed = !active && run.conclusion !== "success";
  const fail = (e: Error) => toast.show(e.message, { tone: "danger" });
  return (
    <View style={[styles.run, { borderColor: tokens.border }]}>
      <Pressable
        onPress={() => setOpen((v) => !v)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={`${run.displayTitle}, ${runLabel(run)}`}
        style={styles.row}
      >
        {open ? <ChevronDown size={14} color={tokens.ink3} /> : <ChevronRight size={14} color={tokens.ink3} />}
        <View style={styles.flex}>
          <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
            {run.displayTitle}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            {run.workflowName} - {run.headBranch} - {ago(run.createdAt)}
          </Text>
        </View>
        <Badge label={runLabel(run)} tone={runTone(run)} lowercase />
      </Pressable>
      <View style={styles.runActions}>
        {active ? (
          <IconButton
            icon={<Square size={16} color={tokens.ink2} />}
            accessibilityLabel={`Cancel run ${run.number}`}
            onPress={() => cancel.mutate({ session: sessionId, id: run.databaseId }, { onError: fail })}
          />
        ) : (
          <IconButton
            icon={<RotateCcw size={16} color={tokens.ink2} />}
            accessibilityLabel={failed ? `Re-run failed jobs of run ${run.number}` : `Re-run run ${run.number}`}
            onPress={() => rerun.mutate({ session: sessionId, id: run.databaseId, failed_only: failed }, { onError: fail })}
          />
        )}
        {!active ? (
          <IconButton
            icon={<Trash2 size={16} color={tokens.ink2} />}
            accessibilityLabel={`Delete run ${run.number}`}
            onPress={() => onDelete(run)}
          />
        ) : null}
        <IconButton
          icon={<ExternalLink size={16} color={tokens.ink2} />}
          accessibilityLabel="Open run on GitHub"
          onPress={() => void Linking.openURL(run.url).catch(() => undefined)}
        />
      </View>
      {open ? <RunJobs sessionId={sessionId} runId={run.databaseId} /> : null}
    </View>
  );
}

export function ActionsDock({ sessionId }: { sessionId: string | null }) {
  const tokens = useTokens();
  const toast = useToast();
  const direct = usePrAvailable();
  const [pinned, togglePin] = usePinned();
  const [filter, setFilter] = useState<string | null>(null);
  const [dispatching, setDispatching] = useState<WorkflowRow | null>(null);
  const [deleting, setDeleting] = useState<WorkflowRunRow | null>(null);
  const workflows = useWorkflows(sessionId);
  const runs = useWorkflowRuns(sessionId, filter ?? undefined);
  const remove = useRunAction("delete");
  const ordered = useMemo(() => orderWorkflows(workflows.data?.items ?? [], pinned), [workflows.data, pinned]);

  if (!sessionId) return <EmptyState icon={Workflow} message="Open a session to see its repository's Actions here." />;
  if (!direct) return <EmptyState icon={Workflow} message="GitHub Actions need a direct connection to the host." />;
  const unavailable = workflows.data && !workflows.data.available ? workflows.data.error : null;
  if (unavailable) return <EmptyState icon={Workflow} message={unavailable} />;
  if (workflows.error) return <EmptyState icon={Workflow} message={workflows.error.message} />;

  return (
    <ScrollView style={{ backgroundColor: tokens.bg0 }} contentContainerStyle={styles.content}>
      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Workflows</Text>
      {workflows.isPending ? <ActivityIndicator color={tokens.accent} /> : null}
      {ordered.length === 0 && !workflows.isPending ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No workflows in this repository.</Text>
      ) : null}
      {ordered.map((workflow) => {
        const isPinned = pinned.includes(workflow.path);
        const selected = filter === workflow.path;
        return (
          <View key={workflow.id} style={[styles.workflow, { borderColor: selected ? tokens.accent : tokens.border }]}>
            <Pressable
              onPress={() => setFilter(selected ? null : workflow.path)}
              accessibilityRole="button"
              accessibilityState={{ selected }}
              accessibilityLabel={`${selected ? "Show all runs instead of" : "Filter runs to"} ${workflow.name}`}
              style={styles.flex}
            >
              <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
                {workflow.name}
              </Text>
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                {workflow.path}
                {workflow.state !== "active" ? ` - ${workflow.state.replace(/_/g, " ")}` : ""}
              </Text>
            </Pressable>
            <IconButton
              icon={<Pin size={16} color={isPinned ? tokens.accent : tokens.ink3} fill={isPinned ? tokens.accent : "none"} />}
              accessibilityLabel={isPinned ? `Unpin ${workflow.name}` : `Pin ${workflow.name}`}
              onPress={() => togglePin(workflow.path)}
            />
            <IconButton
              icon={<Play size={16} color={tokens.ink2} />}
              accessibilityLabel={`Run ${workflow.name}`}
              onPress={() => setDispatching(workflow)}
              disabled={workflow.state !== "active"}
            />
          </View>
        );
      })}

      <Text style={[typeScale.meta, styles.runsHead, { color: tokens.ink2 }]}>
        Runs{filter ? " (filtered)" : ""}
      </Text>
      {runs.isPending ? <ActivityIndicator color={tokens.accent} /> : null}
      {runs.error ? <Text style={[typeScale.sub, { color: tokens.danger }]}>{runs.error.message}</Text> : null}
      {runs.data && runs.data.items.length === 0 ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No runs yet.</Text>
      ) : null}
      {runs.data?.items.map((run) => (
        <RunRow key={run.databaseId} sessionId={sessionId} run={run} onDelete={setDeleting} />
      ))}

      <DispatchSheet sessionId={sessionId} workflow={dispatching} onClose={() => setDispatching(null)} />
      <ConfirmDialog
        visible={deleting != null}
        title="Delete this run?"
        message={deleting ? `${deleting.displayTitle} (run ${deleting.number}) and its logs are removed from GitHub.` : undefined}
        confirmLabel="Delete"
        destructive
        loading={remove.isPending}
        onConfirm={() => {
          const target = deleting;
          setDeleting(null);
          if (target) {
            remove.mutate(
              { session: sessionId, id: target.databaseId },
              { onError: (e) => toast.show(e.message, { tone: "danger" }) },
            );
          }
        }}
        onCancel={() => setDeleting(null)}
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space12, gap: space.space8 },
  flex: { flex: 1 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  runsHead: { marginTop: space.space12 },
  workflow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingLeft: space.space12,
    paddingVertical: space.space4,
  },
  run: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space8, gap: space.space4 },
  runActions: { flexDirection: "row", justifyContent: "flex-end", gap: space.space4 },
  jobs: { gap: space.space8, paddingTop: space.space4 },
  job: { gap: space.space2 },
  step: { paddingLeft: space.space16 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  dotSmall: { width: 6, height: 6, borderRadius: 3 },
  sheet: { padding: space.space16, gap: space.space12 },
  switchRow: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  optionBlock: { gap: space.space4 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
