// Workflow Run + Result screen. Consumes `snapshot.workflow` + the workflow's agent rows
// (subagents whose `phase` is set). While the run is active it shows the phase timeline with
// live agent rows; on compact, tapping a row pushes an internal drill-in pane, on medium+ the
// drill-in folds beside the timeline (desktop two-column). Once the run is finished (present
// but not active) it shows the result view. Stop → confirm → interrupt (whole run — the wire
// has no per-agent or pause control, so none is drawn).
import { ArrowLeft, Check, Workflow as WorkflowIcon, WifiOff, X } from "lucide-react-native";
import { router } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { Button } from "../../../components/ds/Button";
import { ConfirmDialog } from "../../../components/ds/ConfirmDialog";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { useToast } from "../../../components/ds/ToastHost";
import { AgentDrill } from "../../../components/workflow/AgentDrill";
import { NarrationLog } from "../../../components/workflow/NarrationLog";
import { PhaseTimeline } from "../../../components/workflow/PhaseTimeline";
import { PipelineLane } from "../../../components/workflow/PipelineLane";
import { ProgressHeader } from "../../../components/workflow/ProgressHeader";
import { WorkflowResult } from "../../../components/workflow/WorkflowResult";
import {
  groupByPhase,
  totalCost,
  useElapsedSeconds,
  workflowRows,
  workflowTitle,
} from "../../../components/workflow/format";
import type { SnapshotSubagent } from "../../../lib/ws";
import { useSessionCtx } from "../../../lib/sessionContext";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../../theme/typography";
import { useBreakpoint } from "../../../theme/useBreakpoint";

export default function WorkflowRunScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { snapshot, snapshotTimedOut, send } = useSessionCtx();
  const { isCompact } = useBreakpoint();

  const workflow = snapshot?.workflow ?? null;
  const active = workflow?.active ?? false;
  const rows = useMemo(() => workflowRows(snapshot?.subagents ?? []), [snapshot?.subagents]);
  const groups = useMemo(() => groupByPhase(rows, workflow?.phases ?? []), [rows, workflow?.phases]);
  const doneCount = useMemo(() => rows.filter((r) => r.done).length, [rows]);
  const cost = useMemo(() => totalCost(rows), [rows]);
  const elapsed = useElapsedSeconds(active);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmStop, setConfirmStop] = useState(false);

  const onSelectAgent = useCallback((agent: SnapshotSubagent) => setSelectedId(agent.id), []);
  const clearSelection = useCallback(() => setSelectedId(null), []);
  const back = useCallback(() => router.back(), []);

  const onStop = useCallback(() => setConfirmStop(true), []);
  const confirmInterrupt = useCallback(() => {
    setConfirmStop(false);
    if (!send({ kind: "interrupt" })) toast.show("not sent — reconnect and try again", { tone: "danger" });
  }, [send, toast]);

  // ---- loading / empty ----------------------------------------------------
  if (snapshot == null) {
    return (
      <Screen edges={["left", "right", "bottom"]}>
        {snapshotTimedOut ? (
          <EmptyState icon={WifiOff} message="can't reach this session — it may not exist, or the server is unreachable" />
        ) : (
          <View style={styles.loading}>
            <ActivityIndicator color={tokens.accent} />
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>loading workflow…</Text>
          </View>
        )}
      </Screen>
    );
  }

  if (!workflow) {
    return (
      <Screen edges={["left", "right", "bottom"]}>
        <EmptyState
          icon={WorkflowIcon}
          message="No workflow is running in this session."
          action={<Button label="Browse workflows" variant="secondary" onPress={() => router.replace(`/session/${snapshot.session_id}/workflows` as never)} />}
        />
      </Screen>
    );
  }

  const title = workflowTitle(workflow);

  // ---- finished: result view ---------------------------------------------
  if (!active) {
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        <ResultHeader title={title} ok={workflow.finished_ok} cost={cost} onBack={back} />
        <WorkflowResult workflow={workflow} rows={rows} />
        <NarrationLog logs={workflow.logs} />
      </Screen>
    );
  }

  const timeline = (
    <>
      <ProgressHeader
        title={title}
        active={active}
        finishedOk={workflow.finished_ok}
        doneCount={doneCount}
        totalCount={rows.length}
        cost={cost}
        elapsedSeconds={elapsed}
        onBack={back}
        onStop={onStop}
      />
      <View style={styles.timelineBody}>
        <PhaseTimeline groups={groups} selectedId={isCompact ? null : selectedId} onSelectAgent={onSelectAgent} />
        <PipelineLane groups={groups} />
        <NarrationLog logs={workflow.logs} />
      </View>
    </>
  );

  // ---- compact: timeline, or a pushed drill-in pane -----------------------
  if (isCompact) {
    const drillAgent = rows.find((r) => r.id === selectedId) ?? null;
    return (
      <Screen edges={["left", "right", "bottom"]} scroll contentContainerStyle={styles.column}>
        {drillAgent ? (
          <AgentDrill agent={drillAgent} active={active} onBack={clearSelection} onStop={onStop} />
        ) : (
          timeline
        )}
        <StopConfirm visible={confirmStop} onConfirm={confirmInterrupt} onCancel={() => setConfirmStop(false)} />
      </Screen>
    );
  }

  // ---- medium+: two-column (timeline left ~55%, drill-in right) ------------
  const defaultAgent = rows.find((r) => !r.done) ?? rows[0] ?? null;
  const paneAgent = rows.find((r) => r.id === selectedId) ?? defaultAgent;

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.split}>
      <ScrollView style={styles.leftPane} contentContainerStyle={styles.leftContent}>
        {timeline}
      </ScrollView>
      <View style={[styles.rightPane, { borderLeftColor: tokens.border }]}>
        <ScrollView contentContainerStyle={styles.rightContent}>
          <AgentDrill agent={paneAgent} active={active} onStop={onStop} />
        </ScrollView>
      </View>
      <StopConfirm visible={confirmStop} onConfirm={confirmInterrupt} onCancel={() => setConfirmStop(false)} />
    </Screen>
  );
}

function StopConfirm({ visible, onConfirm, onCancel }: { visible: boolean; onConfirm: () => void; onCancel: () => void }) {
  return (
    <ConfirmDialog
      visible={visible}
      title="Stop this workflow?"
      message="Interrupt halts the whole run — every phase and running agent stops. This can't be undone."
      confirmLabel="Stop run"
      cancelLabel="Keep running"
      destructive
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  );
}

function ResultHeader({ title, ok, cost, onBack }: { title: string; ok: boolean | null; cost: number; onBack: () => void }) {
  const tokens = useTokens();
  const bg = ok === false ? tokens.dangerBg : tokens.successBg;
  const ink = ok === false ? tokens.danger : tokens.success;
  return (
    <View style={styles.resultHeader}>
      <Pressable onPress={onBack} accessibilityRole="button" accessibilityLabel="Back to workflows" hitSlop={6} style={styles.resultBack}>
        <ArrowLeft size={20} strokeWidth={2} color={tokens.ink2} />
      </Pressable>
      <View style={[styles.resultMedallion, { backgroundColor: bg }]}>
        {ok === false ? <X size={11} strokeWidth={3} color={ink} /> : <Check size={11} strokeWidth={3} color={ink} />}
      </View>
      <Text style={[typeScale.headingBold, styles.resultTitle, { color: tokens.ink }]} numberOfLines={1}>
        {`${title} · result`}
      </Text>
      <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]}>{formatCost(cost)}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  column: { width: "100%", maxWidth: 760, alignSelf: "center", paddingTop: space.space8, paddingBottom: space.space32 },
  loading: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space12 },
  timelineBody: { marginTop: space.space4 },
  split: { flex: 1, flexDirection: "row" },
  leftPane: { flex: 0.55, minWidth: 0 },
  leftContent: { paddingTop: space.space8, paddingBottom: space.space32, paddingRight: space.space24 },
  rightPane: { flex: 0.45, minWidth: 0, borderLeftWidth: StyleSheet.hairlineWidth },
  rightContent: { paddingTop: space.space8, paddingBottom: space.space32, paddingLeft: space.space24 },
  resultHeader: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space4 },
  resultBack: { width: 28, height: 44, marginLeft: -6, alignItems: "center", justifyContent: "center" },
  resultMedallion: { width: 18, height: 18, borderRadius: 9, alignItems: "center", justifyContent: "center" },
  resultTitle: { flex: 1, fontSize: 19 },
});
