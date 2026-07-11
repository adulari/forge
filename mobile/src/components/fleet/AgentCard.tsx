// DESIGN_SYSTEM.md §6 `AgentCard`: bot icon + agent name (heading, accent), task sub, model
// Badge, cost Metric, `last` line (codeSmall, 2 lines, bg0 well); done: 0.65 opacity + check.
// DESIGN_ELEVATION.md Move 1 (thermal identity): a running (non-done) agent carries the
// subtle HeatEdge — the object is genuinely "live". Move 3: feature-card radius 16, mono
// cost metric. Per Move 2, Agent is one of the few things that IS a Card (a discrete object),
// not a de-boxed list row.
import { Bot, Check } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import type { SnapshotSubagent } from "../../lib/ws";
import { Badge } from "../ds/Badge";
import { Card } from "../ds/Card";
import { CostMetric } from "../ds/CostMetric";
import { HeatEdge } from "../ds/HeatEdge";

const ICON_SIZE = 20;
const ICON_STROKE = 1.75;
const DONE_OPACITY = 0.65;

export interface AgentCardProps {
  agent: SnapshotSubagent;
}

function AgentCardBase({ agent }: AgentCardProps) {
  const tokens = useTokens();
  const running = !agent.done;

  return (
    <Card variant="feature" style={agent.done ? styles.done : undefined}>
      <HeatEdge state={running ? "busy" : false} />
      <View style={styles.header}>
        <Bot size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.accent} />
        <Text style={[typeScale.heading, styles.name, { color: tokens.accent }]} numberOfLines={1}>
          {agent.agent}
        </Text>
        {agent.done ? <Check size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.success} /> : null}
      </View>

      <Text style={[typeScale.sub, styles.task, { color: tokens.ink2 }]} numberOfLines={2}>
        {agent.task}
      </Text>

      <View style={styles.metaRow}>
        {agent.model ? <Badge label={agent.model} tone="neutral" /> : null}
        <CostMetric valueUsd={agent.cost} />
      </View>

      {agent.last ? (
        <View style={[styles.lastWell, { backgroundColor: tokens.bg0 }]}>
          <Text style={[typeScale.codeSmall, { color: tokens.ink2 }]} numberOfLines={2}>
            {agent.last}
          </Text>
        </View>
      ) : null}
    </Card>
  );
}

export const AgentCard = React.memo(AgentCardBase, (prev, next) => {
  const a = prev.agent;
  const b = next.agent;
  return (
    a.agent === b.agent &&
    a.task === b.task &&
    a.model === b.model &&
    a.last === b.last &&
    a.done === b.done &&
    a.cost === b.cost
  );
});

const styles = StyleSheet.create({
  done: { opacity: DONE_OPACITY },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1 },
  task: { marginTop: space.space4 },
  metaRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginTop: space.space12,
  },
  lastWell: {
    marginTop: space.space12,
    borderRadius: 4,
    padding: space.space8,
  },
});
