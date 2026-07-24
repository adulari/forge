// Machined FloorTile: a bordered card (matching D/M Floor's live-tail tiles) — StatusDot +
// title + a mono LIVE badge (accent, real socket state) replaces the old plain connection
// word; assistant/tool tail text; inline permission/question card when one is pending; a
// single mono meta footer line (tasks · ctx% · cost · queued · host), all technical figures
// in Geist Mono per the mono-discipline rule.
//
// The design's mockup renders assistant prose and a separate bordered mono "tool output" block
// beneath it. That split needs per-line provenance, which protocol v9's `transcript_rows`
// finally carries (`kind: user|assistant|tool|system`, plus the `tool` name and an `ok`/`failed`
// `meta` on a result row) — `splitTail` below derives both from it. A pre-v9 host sends no
// `transcript_rows` at all, so the tile falls back to exactly the old behaviour: one prose line
// off the flat `transcript`, no tool block.
import { router } from "expo-router";
import { Ellipsis, ListX, Pause, WifiOff } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useSessionSocket, type Snapshot, type TranscriptRow } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { PermissionCard } from "../cards/PermissionCard";
import { QuestionCard } from "../cards/QuestionCard";
import { ContextGauge } from "../ds/ContextGauge";
import { CostMetric } from "../ds/CostMetric";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";

export interface FloorTileProps { row: SessionRow; active: boolean; }

interface Tail {
  /** The newest prose line (assistant/user/system) — never a tool line. */
  prose: string | null;
  /** The newest tool row, but only when it is newer than `prose` — a tool block that a later
   * assistant line has already superseded is stale and isn't the tile's live tail. */
  tool: TranscriptRow | null;
}

/**
 * Split the live tail into prose + the tool block the design draws beneath it. Returns
 * `{prose, tool: null}` against a pre-v9 host (no `transcript_rows`), which reads exactly like
 * the old flat-`transcript` tail.
 */
export function splitTail(snapshot: Snapshot | null): Tail {
  const rows = snapshot?.transcript_rows;
  if (rows == null || rows.length === 0) {
    return { prose: snapshot?.transcript.at(-1) ?? null, tool: null };
  }
  let proseIndex = -1;
  let toolIndex = -1;
  for (let i = rows.length - 1; i >= 0 && (proseIndex === -1 || toolIndex === -1); i -= 1) {
    if (rows[i].kind === "tool") {
      if (toolIndex === -1) toolIndex = i;
    } else if (proseIndex === -1) {
      proseIndex = i;
    }
  }
  return {
    prose: proseIndex === -1 ? null : rows[proseIndex].text,
    tool: toolIndex > proseIndex ? rows[toolIndex] : null,
  };
}

function connectionLabel(state: string) {
  if (state === "open") return "LIVE";
  if (state === "unreachable") return "unreachable";
  if (state === "closed") return "closed";
  if (state === "idle") return "idle";
  return "reconnecting";
}

function FloorTileBase({ row, active }: FloorTileProps) {
  const tokens = useTokens();
  const { baseUrl, servers, activeServerId } = useAuth();
  const hostLabel = servers.find((s) => s.id === activeServerId)?.name ?? null;
  const { snapshot, connectionState, send } = useSessionSocket(baseUrl, active ? row.id : null);
  const [actionsVisible, setActionsVisible] = useState(false);
  const title = snapshot?.title || row.title || `session ${row.id.slice(0, 8)}`;
  const waiting = snapshot?.permission_prompt != null || snapshot?.question != null || row.waiting;
  const busy = snapshot?.busy ?? row.busy;
  const { prose, tool } = splitTail(snapshot ?? null);
  // A live stream is newer than anything already committed to the transcript, so it wins the
  // prose slot — but the tool block beneath it stays, since that is what the model is streaming
  // *about*. The "warming socket…" filler is only for a tile with nothing at all to show: a
  // turn that opens on tool activity has a real tail already, just not a prose one.
  const tail = snapshot?.streaming || prose || (tool ? null : "warming socket…");
  const tasksDone = snapshot?.tasks.filter((task) => task.status === "done").length ?? 0;
  const taskCount = snapshot?.tasks.length ?? 0;
  const state = waiting ? "waiting" : busy ? "busy" : "idle";
  const live = connectionState === "open";
  const label = connectionLabel(connectionState);
  const open = useCallback(() => router.push(`/session/${row.id}`), [row.id]);

  return <>
    <Pressable onPress={open} onLongPress={() => setActionsVisible(true)} style={[styles.tile, { backgroundColor: waiting ? tokens.selection : tokens.bg2, borderColor: tokens.border }]} accessibilityRole="button" accessibilityLabel={`Open ${title}`}>
      <View style={styles.header}>
        <StatusDot state={state} />
        <Text style={[typeScale.bodyBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>{title}</Text>
        <Text style={[typeScale.meta, styles.mono, { color: connectionState === "unreachable" ? tokens.danger : live ? tokens.accent : tokens.ink3, fontFamily: live ? monoFamily.bold : monoFamily.regular }]}>{label}</Text>
        <IconButton icon={<Ellipsis size={18} strokeWidth={1.75} color={tokens.ink3} />} onPress={() => setActionsVisible(true)} accessibilityLabel={`Actions for ${title}`} />
      </View>
      {tail ? <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={3}>{tail}</Text> : null}
      {tool ? <ToolOutputBlock row={tool} /> : null}
      {snapshot?.permission_prompt != null ? <PermissionCard prompt={snapshot.permission_prompt} diff={snapshot.diff} promptSeq={snapshot.prompt_seq} send={send} /> : null}
      {snapshot?.question != null ? <QuestionCard question={snapshot.question} options={snapshot.question_options} allowOther={snapshot.question_allow_other} promptSeq={snapshot.prompt_seq} send={send} /> : null}
      {snapshot?.subagents.length ? <View style={styles.subagents}>{snapshot.subagents.slice(0, 3).map((agent) => <Text key={agent.agent} style={[typeScale.meta, styles.subagent, { color: tokens.ink3 }]} numberOfLines={1}>{agent.agent} · {agent.model ?? "—"} · {agent.last}</Text>)}</View> : null}
      <View style={styles.footer}>
        {taskCount > 0 ? <Text style={[typeScale.monoMeta, tabularNums, styles.mono, { color: tokens.ink3 }]}>{tasksDone}/{taskCount} tasks</Text> : null}
        {snapshot?.context_limit != null ? <ContextGauge used={snapshot.context_tokens} total={snapshot.context_limit} compact ctxLabel /> : null}
        <CostMetric valueUsd={snapshot?.cost_usd ?? row.cost_usd} />
        {snapshot?.queued.length ? <Text style={[typeScale.monoMeta, tabularNums, styles.mono, { color: tokens.warn }]}>{snapshot.queued.length} queued</Text> : null}
        {hostLabel ? <Text style={[typeScale.monoMeta, tabularNums, styles.mono, { color: tokens.ink4 }]}>{hostLabel}</Text> : null}
      </View>
    </Pressable>
    <Sheet visible={actionsVisible} onClose={() => setActionsVisible(false)} accessibilityLabel="Floor tile actions"><View style={styles.sheet}>{busy ? <ListRow title="Pull from the fire" leading={<Pause size={20} color={tokens.danger} />} onPress={() => send({ kind: "interrupt" })} /> : null}{(snapshot?.queued ?? []).map((text, index) => <ListRow key={`${index}:${text}`} title={`Dequeue: ${text}`} leading={<ListX size={20} color={tokens.ink2} />} onPress={() => send({ kind: "dequeue", index, text })} />)}{connectionState === "unreachable" ? <ListRow title="Socket unreachable" leading={<WifiOff size={20} color={tokens.danger} />} showSeparator={false} /> : null}</View></Sheet>
  </>;
}

// The design's separate mono tool-output block: hairline box on bg0, the tool name as its
// header, the run's own text beneath. `meta` is only ever set on a RESULT row ("ok"/"failed"),
// so the outcome tag renders exactly when the daemon actually has an outcome — a call row
// still in flight gets no tag rather than a neutral placeholder.
function ToolOutputBlock({ row }: { row: TranscriptRow }) {
  const tokens = useTokens();
  const failed = row.meta === "failed";
  return (
    <View style={[styles.toolBlock, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      {row.tool || row.meta ? (
        <View style={styles.toolHead}>
          {row.tool ? (
            <Text style={[typeScale.monoMeta, styles.toolName, { color: tokens.ink3 }]} numberOfLines={1}>{row.tool}</Text>
          ) : null}
          {row.meta ? (
            <Text style={[typeScale.monoMeta, tabularNums, { color: failed ? tokens.danger : tokens.success }]}>{row.meta}</Text>
          ) : null}
        </View>
      ) : null}
      <Text style={[typeScale.monoMeta, { color: failed ? tokens.danger : tokens.ink2 }]} numberOfLines={3}>{row.text}</Text>
    </View>
  );
}

export const FloorTile = React.memo(FloorTileBase, (a, b) => a.active === b.active && a.row.id === b.row.id && a.row.title === b.row.title && a.row.cost_usd === b.row.cost_usd && a.row.busy === b.row.busy && a.row.waiting === b.row.waiting && a.row.last_activity === b.row.last_activity);
const styles = StyleSheet.create({
  tile: { position: "relative", borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: space.space8, overflow: "hidden" },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  mono: { fontFamily: monoFamily.regular },
  footer: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: space.space8 },
  toolBlock: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: space.space8, gap: space.space4 },
  toolHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  toolName: { flex: 1 },
  subagents: { gap: space.space4 },
  subagent: { opacity: 0.7 },
  sheet: { paddingHorizontal: space.space4, paddingBottom: space.space16 },
});
