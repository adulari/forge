import { router } from "expo-router";
import { Ellipsis, ListX, Pause, WifiOff } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useSessionSocket } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { PermissionCard } from "../cards/PermissionCard";
import { QuestionCard } from "../cards/QuestionCard";
import { ContextGauge } from "../ds/ContextGauge";
import { CostMetric } from "../ds/CostMetric";
import { HeatEdge } from "../ds/HeatEdge";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";

export interface FloorTileProps { row: SessionRow; active: boolean; }
function connectionLabel(state: string) { return state === "open" ? "live" : state === "unreachable" ? "unreachable" : "reconnecting"; }

function FloorTileBase({ row, active }: FloorTileProps) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const { snapshot, connectionState, send } = useSessionSocket(baseUrl, active ? row.id : null);
  const [actionsVisible, setActionsVisible] = useState(false);
  const title = snapshot?.title || row.title || `session ${row.id.slice(0, 8)}`;
  const waiting = snapshot?.permission_prompt != null || snapshot?.question != null || row.waiting;
  const busy = snapshot?.busy ?? row.busy;
  const tail = snapshot?.streaming || snapshot?.transcript.at(-1) || "warming socket…";
  const tasksDone = snapshot?.tasks.filter((task) => task.status === "done").length ?? 0;
  const taskCount = snapshot?.tasks.length ?? 0;
  const state = waiting ? "waiting" : busy ? "busy" : "idle";
  const open = useCallback(() => router.push(`/session/${row.id}`), [row.id]);

  return <>
    <Pressable onPress={open} onLongPress={() => setActionsVisible(true)} style={[styles.tile, { backgroundColor: waiting ? tokens.selection : tokens.bg2, borderColor: tokens.border }]} accessibilityRole="button" accessibilityLabel={`Open ${title}`}>
      <HeatEdge state={waiting ? "waiting" : busy ? "busy" : false} />
      <View style={styles.header}><StatusDot state={state} /><Text style={[typeScale.bodyBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>{title}</Text><Text style={[typeScale.meta, { color: connectionState === "unreachable" ? tokens.danger : tokens.ink3 }]}>{connectionLabel(connectionState)}</Text><IconButton icon={<Ellipsis size={18} strokeWidth={1.75} color={tokens.ink3} />} onPress={() => setActionsVisible(true)} accessibilityLabel={`Actions for ${title}`} /></View>
      <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={3}>{tail}</Text>
      {taskCount > 0 ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{tasksDone}/{taskCount} tasks</Text> : null}
      {snapshot?.context_limit != null ? <ContextGauge used={snapshot.context_tokens} total={snapshot.context_limit} compact /> : null}
      <View style={styles.metrics}><CostMetric valueUsd={snapshot?.cost_usd ?? row.cost_usd} />{snapshot?.queued.length ? <Text style={[typeScale.meta, { color: tokens.warn }]}>{snapshot.queued.length} queued</Text> : null}</View>
      {snapshot?.subagents.length ? <View style={styles.subagents}>{snapshot.subagents.slice(0, 3).map((agent) => <Text key={agent.agent} style={[typeScale.meta, styles.subagent, { color: tokens.ink3 }]} numberOfLines={1}>{agent.agent} · {agent.model ?? "—"} · {agent.last}</Text>)}</View> : null}
      {snapshot?.permission_prompt != null ? <PermissionCard prompt={snapshot.permission_prompt} diff={snapshot.diff} promptSeq={snapshot.prompt_seq} send={send} /> : null}
      {snapshot?.question != null ? <QuestionCard question={snapshot.question} options={snapshot.question_options} allowOther={snapshot.question_allow_other} promptSeq={snapshot.prompt_seq} send={send} /> : null}
    </Pressable>
    <Sheet visible={actionsVisible} onClose={() => setActionsVisible(false)} accessibilityLabel="Floor tile actions"><View style={styles.sheet}>{busy ? <ListRow title="Pull from the fire" leading={<Pause size={20} color={tokens.danger} />} onPress={() => send({ kind: "interrupt" })} /> : null}{(snapshot?.queued ?? []).map((text, index) => <ListRow key={`${index}:${text}`} title={`Dequeue: ${text}`} leading={<ListX size={20} color={tokens.ink2} />} onPress={() => send({ kind: "dequeue", index, text })} />)}{connectionState === "unreachable" ? <ListRow title="Socket unreachable" leading={<WifiOff size={20} color={tokens.danger} />} showSeparator={false} /> : null}</View></Sheet>
  </>;
}

export const FloorTile = React.memo(FloorTileBase, (a, b) => a.active === b.active && a.row.id === b.row.id && a.row.busy === b.row.busy && a.row.waiting === b.row.waiting && a.row.last_activity === b.row.last_activity);
const styles = StyleSheet.create({ tile: { position: "relative", borderWidth: StyleSheet.hairlineWidth, borderRadius: 12, padding: space.space12, gap: space.space8, overflow: "hidden" }, header: { flexDirection: "row", alignItems: "center", gap: space.space8 }, title: { flex: 1 }, metrics: { flexDirection: "row", justifyContent: "space-between", alignItems: "center" }, subagents: { gap: space.space4 }, subagent: { opacity: 0.7 }, sheet: { paddingHorizontal: space.space4, paddingBottom: space.space16 } });
