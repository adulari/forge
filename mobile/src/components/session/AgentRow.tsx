// Hearth Session Agents screen: de-boxed hairline list (core rule 1) — a running subagent
// carries the accent HeatEdge (core rule 3), NOT a Card (superseding the pre-Hearth
// `fleet/AgentCard.tsx`, which is a discrete elevated Card by design — see its own doc
// comment. That component is still used elsewhere; this is a Hearth-only sibling scoped to
// the session Agents segment).
import { Check } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { SnapshotSubagent } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import Animated from "react-native-reanimated";
import { HeatEdge } from "../ds/HeatEdge";

export interface AgentRowProps {
  agent: SnapshotSubagent;
  showSeparator?: boolean;
}

function AgentDot({ running }: { running: boolean }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(running ? "busy" : "idle");
  const color = running ? tokens.accent : tokens.ink3;
  return (
    <View style={styles.dotWrap}>
      {running ? <View style={[styles.dotGlow, { backgroundColor: tokens.dotGlow }]} /> : null}
      <Animated.View style={[styles.dot, { backgroundColor: color }, dotStyle]} />
    </View>
  );
}

function AgentRowBase({ agent, showSeparator = true }: AgentRowProps) {
  const tokens = useTokens();
  const running = !agent.done;

  return (
    <View style={[styles.wrap, agent.done && styles.done]}>
      {running ? <HeatEdge state="busy" /> : null}
      <View style={styles.row}>
        <View style={styles.header}>
          <AgentDot running={running} />
          <Text style={[typeScale.heading, styles.name, { color: tokens.ink }]} numberOfLines={1}>
            {agent.agent}
          </Text>
          {agent.done ? <Check size={14} strokeWidth={2} color={tokens.success} /> : null}
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]}>
            {formatCost(agent.cost)}
          </Text>
        </View>
        {agent.task ? (
          <Text style={[typeScale.sub, styles.task, { color: tokens.ink2 }]} numberOfLines={2}>
            {agent.task}
          </Text>
        ) : null}
        {agent.model || agent.last ? (
          <Text style={[typeScale.monoMeta, styles.last, { color: tokens.ink4 }]} numberOfLines={1}>
            {[agent.model, agent.last].filter(Boolean).join(" · ")}
          </Text>
        ) : null}
      </View>
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

export const AgentRow = React.memo(AgentRowBase);

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  done: { opacity: 0.75 },
  row: { paddingHorizontal: space.space20, paddingVertical: space.space12, gap: space.space4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1 },
  task: { paddingLeft: 17 },
  last: { paddingLeft: 17, marginTop: 2 },
  dotWrap: { width: 8, height: 8, alignItems: "center", justifyContent: "center" },
  dot: { width: 8, height: 8, borderRadius: 4 },
  dotGlow: { position: "absolute", width: 12, height: 12, borderRadius: 6 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space20 },
});
