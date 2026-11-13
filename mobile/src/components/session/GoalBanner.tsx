// The `/goal` run pinned under the session header: the objective, its state (running, paused,
// budget limit, complete, failed, cancelled), turns, token budget and elapsed time, with the
// controls that state allows. Reads `Snapshot.goal`; a host that predates it still shows a
// running goal derived from the CLI's notes, with Stop as its only control.
import { Pause, Pencil, Play, Square, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import {
  budgetFraction,
  contractHeadline,
  type ContractRow,
  contractRows,
  envelopeFraction,
  GOAL_PHASE_LABEL,
  type GoalAction,
  type GoalView,
  goalActions,
  goalInput,
  goalMeta,
  missionGroups,
  missionLine,
  relayLine,
  resolveGoal,
  resumeLine,
} from "../../lib/goalState";
import { haptics } from "../../lib/haptics";
import type { GoalPhase, RemoteInput, Snapshot } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { GoalSheet } from "../chat/GoalSheet";
import { ScreenshotThumb } from "../chat/ScreenshotThumb";
import { MiniPressable } from "../chat/MiniPressable";
import { Card } from "../ds/Card";
import { useClockSeconds } from "../ds/relativeClock";
import { StatusDot } from "../ds/StatusDot";

export { deriveNoteGoal as deriveActiveGoal } from "../../lib/goalState";

export interface GoalBannerProps {
  snapshot: Snapshot | null;
  send: (input: RemoteInput) => boolean;
}

const ACTION_ICON = { pause: Pause, resume: Play, stop: Square, edit: Pencil, clear: X } as const;
const ACTION_LABEL: Record<GoalAction, string> = {
  pause: "Pause",
  resume: "Resume",
  stop: "Stop",
  edit: "Edit",
  clear: "Clear",
};

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);
  return now;
}

function dotState(phase: GoalPhase): StatusDotState {
  if (phase === "active") return "busy";
  if (phase === "paused") return "waiting";
  if (phase === "complete") return "done";
  return "idle";
}

function ActionButton({ action, onPress }: { action: GoalAction; onPress: (action: GoalAction) => void }) {
  const tokens = useTokens();
  const Icon = ACTION_ICON[action];
  const color = action === "stop" ? tokens.danger : action === "resume" ? tokens.accent : tokens.ink2;
  return (
    <MiniPressable
      onPress={() => onPress(action)}
      accessibilityRole="button"
      accessibilityLabel={`${ACTION_LABEL[action]} goal`}
      hitSlop={6}
      style={styles.action}
      testID={`goal-${action}`}
    >
      <Icon size={11} color={color} fill={action === "stop" ? color : "none"} strokeWidth={action === "stop" ? 0 : 2} />
      <Text style={[typeScale.meta, { color }]}>{ACTION_LABEL[action]}</Text>
    </MiniPressable>
  );
}

function GoalBannerBody({ goal, send }: { goal: GoalView; send: GoalBannerProps["send"] }) {
  const tokens = useTokens();
  const [editing, setEditing] = useState(false);
  const live = goal.phase === "active";
  const now = useNow(live && goal.structured);
  const resume = resumeLine(goal, useClockSeconds() * 1000);
  const fraction = budgetFraction(goal);
  const envelopeFill = envelopeFraction(goal);
  const actions = goalActions(goal);
  const ended = !live && goal.phase !== "paused";
  const tone = !ended ? tokens.accent : goal.phase === "complete" ? tokens.success : goal.phase === "cancelled" ? tokens.ink3 : tokens.danger;

  const run = (action: GoalAction) => {
    if (action === "edit") {
      setEditing(true);
      return;
    }
    if (action === "stop") haptics.deny();
    else haptics.select();
    send(goalInput(action));
  };

  const status = goal.pending === "pause" ? "pausing after this turn" : goal.pending === "clear" ? "stopping after this turn" : null;

  return (
    <View style={styles.wrap}>
      <Card style={[styles.card, { borderColor: ended ? tokens.border : tokens.accent }]}>
        <View style={styles.header}>
          <StatusDot state={dotState(goal.phase)} size={6} accessibilityLabel={`goal ${GOAL_PHASE_LABEL[goal.phase]}`} />
          <View style={[styles.dash, { backgroundColor: tone }]} />
          <Text style={[typeScale.section, { color: tone }]} numberOfLines={1}>
            {`Goal · ${GOAL_PHASE_LABEL[goal.phase]}`}
          </Text>
          <View style={styles.spacer} />
          <View style={styles.actions}>
            {actions.map((action) => (
              <ActionButton key={action} action={action} onPress={run} />
            ))}
          </View>
        </View>
        <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={2}>
          {goal.objective}
        </Text>
        {status ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{status}</Text> : null}
        {relayLine(goal) ? (
          <Text style={[styles.meta, { color: tokens.accent }]} numberOfLines={2} testID="goal-relay">
            {relayLine(goal)}
          </Text>
        ) : null}
        {goal.reason && (ended || goal.phase === "paused") ? <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={2}>{goal.reason}</Text> : null}
        {resume ? (
          <Text style={[styles.meta, { color: tokens.accent }]} numberOfLines={1} testID="goal-resume">
            {resume}
          </Text>
        ) : null}
        <Text style={[styles.meta, { color: tokens.ink3 }]} numberOfLines={1} testID="goal-meta">
          {goalMeta(goal, now)}
        </Text>
        {fraction != null ? (
          <View
            style={[styles.track, { backgroundColor: tokens.bg3 }]}
            accessibilityRole="progressbar"
            accessibilityLabel="Goal token budget"
            accessibilityValue={{ min: 0, max: 100, now: Math.round(fraction * 100) }}
          >
            <View style={[styles.fill, { width: `${Math.round(fraction * 100)}%`, backgroundColor: fraction >= 0.9 ? tokens.danger : tokens.accent }]} />
          </View>
        ) : null}
        {goal.envelope && envelopeFill != null ? (
          <>
            <Text
              style={[styles.meta, { color: goal.envelope.spent || goal.envelope.unverified ? tokens.danger : tokens.ink3 }]}
              numberOfLines={1}
              testID="goal-envelope"
            >
              {goal.envelope.unverified ? `${goal.envelope.meter} · unverified` : goal.envelope.meter}
            </Text>
            <View
              style={[styles.track, { backgroundColor: tokens.bg3 }]}
              accessibilityRole="progressbar"
              accessibilityLabel={`Goal envelope, ${goal.envelope.label}`}
              accessibilityValue={{ min: 0, max: 100, now: Math.round(envelopeFill * 100) }}
            >
              <View style={[styles.fill, { width: `${Math.round(envelopeFill * 100)}%`, backgroundColor: envelopeFill >= 0.8 ? tokens.danger : tokens.accent }]} />
            </View>
          </>
        ) : null}
        {contractRows(goal) ? (
          <View style={styles.contract} testID="goal-contract">
            {missionLine(goal) ? (
              <Text style={[typeScale.section, { color: tokens.accent }]} testID="goal-mission">
                {missionLine(goal)}
              </Text>
            ) : null}
            <Text style={[typeScale.section, { color: tokens.ink2 }]}>{contractHeadline(goal)}</Text>
            {missionGroups(goal)
              ? missionGroups(goal)?.map((group) => (
                  <View key={group.index} style={styles.contract} testID={`goal-milestone-${group.index}`}>
                    <Text
                      style={[typeScale.meta, { color: group.state === "upcoming" ? tokens.ink3 : tokens.ink }]}
                      accessibilityLabel={`Milestone ${group.index}, ${group.state}`}
                    >
                      {`Milestone ${group.index} · ${group.title} · ${group.tally}`}
                    </Text>
                    {group.rows.map((row) => (
                      <ContractRowView key={row.id} row={row} />
                    ))}
                  </View>
                ))
              : contractRows(goal)?.map((row) => <ContractRowView key={row.id} row={row} />)}
          </View>
        ) : null}
      </Card>
      <GoalSheet
        visible={editing}
        mode="edit"
        initialValue={goal.objective}
        onClose={() => setEditing(false)}
        onSubmit={(text) => send({ kind: "prompt", text })}
      />
    </View>
  );
}

function ContractRowView({ row }: { row: ContractRow }) {
  const tokens = useTokens();
  return (
    <View style={styles.row} testID={`goal-contract-${row.id}`}>
      <Text
        style={[
          typeScale.meta,
          { color: row.state === "pass" ? tokens.success : row.state === "fail" ? tokens.danger : tokens.ink3 },
        ]}
        accessibilityLabel={row.state}
      >
        {row.mark}
      </Text>
      <View style={styles.rowBody}>
        <Text style={[typeScale.meta, { color: tokens.ink }]}>{`${row.id}. ${row.text}`}</Text>
        <Text style={[styles.meta, { color: tokens.ink3 }]} numberOfLines={1}>{row.command}</Text>
        {row.detail ? (
          <Text style={[styles.meta, { color: row.state === "fail" ? tokens.danger : tokens.ink3 }]} numberOfLines={2}>
            {row.detail}
          </Text>
        ) : null}
        {row.artifact ? <ScreenshotThumb path={row.artifact} /> : null}
      </View>
    </View>
  );
}

export function GoalBanner({ snapshot, send }: GoalBannerProps) {
  const goal = resolveGoal(snapshot);
  if (!goal) return null;
  return <GoalBannerBody goal={goal} send={send} />;
}

const styles = StyleSheet.create({
  wrap: { paddingBottom: space.space8 },
  card: { gap: space.space4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  spacer: { flex: 1 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  action: { flexDirection: "row", alignItems: "center", gap: space.space4, minHeight: 28, paddingHorizontal: space.space8, borderRadius: 6 },
  contract: { gap: space.space4, marginTop: space.space8 },
  row: { flexDirection: "row", gap: space.space8 },
  rowBody: { flex: 1, gap: 1 },
  meta: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
  track: { height: 2, borderRadius: 1, overflow: "hidden", marginTop: space.space4 },
  fill: { height: 2 },
});
