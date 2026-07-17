// Hearth Subagents row: de-boxed hairline row (core rule 1) for one `spawn_agents` child.
// A running child carries the accent HeatEdge (core rule 3); a failed child (done && !ok)
// carries the waiting/danger HeatEdge and paints its tail in danger; a settled child dims.
// Optional `onPress`/`expanded` drive the inline expandable detail owned by SubagentsPanel
// (compact layout). This supersedes the pre-Hearth `fleet/AgentCard.tsx` (a discrete Card).
import { Check } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { SnapshotSubagent } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import { HeatEdge } from "../ds/HeatEdge";
import { StatusDot } from "../ds/StatusDot";

export interface AgentRowProps {
  agent: SnapshotSubagent;
  showSeparator?: boolean;
  /** Inline detail open — unclamps the task + live tail (compact expandable detail). */
  expanded?: boolean;
  /** Makes the row a button that toggles its inline detail. Omit for a static row. */
  onPress?: () => void;
}

type RowState = "running" | "failed" | "done";

export function rowStateOf(agent: SnapshotSubagent): RowState {
  if (!agent.done) return "running";
  return agent.ok ? "done" : "failed";
}

function AgentRowBase({ agent, showSeparator = true, expanded = false, onPress }: AgentRowProps) {
  const tokens = useTokens();
  const state = rowStateOf(agent);
  const running = state === "running";
  const failed = state === "failed";
  const done = state === "done";

  const tailColor = failed ? tokens.danger : done ? tokens.ink4 : tokens.ink3;

  const body = (
    <View style={styles.row}>
      <View style={styles.header}>
        {running ? (
          <StatusDot state="busy" accessibilityLabel={`${agent.agent}: running`} />
        ) : failed ? (
          <View style={[styles.failDot, { backgroundColor: tokens.danger }]} />
        ) : (
          <StatusDot state="done" accessibilityLabel={`${agent.agent}: done`} />
        )}
        <Text style={[typeScale.bodyBold, styles.name, { color: done ? tokens.ink2 : tokens.ink }]} numberOfLines={1}>
          {agent.agent}
        </Text>
        {agent.model ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            {agent.model}
          </Text>
        ) : null}
        {done ? <Check size={14} strokeWidth={2} color={tokens.success} /> : null}
        <Text style={[typeScale.monoMeta, tabularNums, { color: failed ? tokens.ink3 : tokens.success }]}>
          {formatCost(agent.cost)}
        </Text>
      </View>
      {agent.task ? (
        <Text
          style={[typeScale.sub, styles.indent, { color: tokens.ink2 }]}
          numberOfLines={expanded ? undefined : 1}
        >
          {agent.task}
        </Text>
      ) : null}
      {agent.last ? (
        <Text
          style={[typeScale.monoMeta, styles.indent, styles.tail, { color: tailColor }]}
          numberOfLines={expanded ? undefined : 2}
        >
          {agent.last}
        </Text>
      ) : null}
    </View>
  );

  return (
    <View style={[styles.wrap, done && styles.done]}>
      {running ? <HeatEdge state="busy" /> : failed ? <HeatEdge state="waiting" /> : null}
      {onPress ? (
        <Pressable
          onPress={onPress}
          accessibilityRole="button"
          accessibilityState={{ expanded }}
          accessibilityLabel={`Subagent ${agent.agent}`}
        >
          {body}
        </Pressable>
      ) : (
        body
      )}
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

export const AgentRow = React.memo(AgentRowBase);

const EDGE_GUTTER = 16;

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  done: { opacity: 0.7 },
  row: {
    paddingLeft: EDGE_GUTTER,
    paddingRight: space.space16,
    paddingVertical: space.space12,
    gap: space.space4,
  },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1 },
  indent: { paddingLeft: EDGE_GUTTER },
  tail: { marginTop: space.space2 },
  failDot: { width: 8, height: 8, borderRadius: 4 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: EDGE_GUTTER },
});
