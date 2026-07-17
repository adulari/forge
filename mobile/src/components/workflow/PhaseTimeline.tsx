// New pattern 1 (Phase timeline) + pattern 2 (Agent live row). Vertical phases with state
// medallions — done disc+check, running disc+emberdot with a heat edge in the left gutter,
// pending ring — and the workflow's agent rows nested beneath their phase. Rows are the
// honest wire rows only: emberdot, name, model id, live tail, cost; a failed row takes the
// waiting edge + danger tail. No retry/skip affordance exists on the wire, so none is drawn.
import { Check } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { SnapshotSubagent } from "../../lib/ws";
import { useEmberdot, useForgeline } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { HeatEdge } from "../ds/HeatEdge";
import { isFailed, type PhaseGroup } from "./format";

function Emberdot({ kind }: { kind: "busy" | "waiting" | "idle" }) {
  const tokens = useTokens();
  const { dotStyle, ringStyle } = useEmberdot(kind);
  const color = kind === "busy" ? tokens.accent : kind === "waiting" ? tokens.danger : tokens.ink3;
  return (
    <View style={styles.dotWrap}>
      {kind === "waiting" ? (
        <Animated.View style={[styles.dotRing, { borderColor: tokens.danger }, ringStyle]} />
      ) : null}
      <Animated.View style={[styles.dot, { backgroundColor: color }, dotStyle]} />
    </View>
  );
}

function Medallion({ state }: { state: PhaseGroup["state"] }) {
  const tokens = useTokens();
  if (state === "done") {
    return (
      <View style={[styles.medallion, { backgroundColor: tokens.successBg }]}>
        <Check size={11} strokeWidth={3} color={tokens.success} />
      </View>
    );
  }
  if (state === "running") {
    return (
      <View style={[styles.medallion, { backgroundColor: tokens.selection }]}>
        <Emberdot kind="busy" />
      </View>
    );
  }
  return <View style={[styles.medallion, styles.medallionPending, { borderColor: tokens.borderStrong }]} />;
}

function phaseSublabel(group: PhaseGroup): string {
  if (group.state === "done") return `${group.rows.length} agents · ${formatCost(group.cost)}`;
  if (group.state === "running") return `${group.runningCount} running · ${group.doneCount} done`;
  return "pending";
}

export function AgentLiveRow({
  agent,
  selected,
  showSeparator,
  onPress,
}: {
  agent: SnapshotSubagent;
  selected: boolean;
  showSeparator: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const failed = isFailed(agent);
  const running = !agent.done;
  const dotKind = failed ? "waiting" : running ? "busy" : "idle";

  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`Agent ${agent.agent}${failed ? ", failed" : running ? ", running" : ", done"}`}
      style={[
        styles.agentWrap,
        selected ? { backgroundColor: tokens.selection, borderRadius: radii.radius8 } : null,
      ]}
    >
      {failed ? <HeatEdge state="waiting" /> : null}
      <View style={styles.agentRow}>
        <View style={styles.agentHeader}>
          <Emberdot kind={dotKind} />
          <Text style={[styles.agentName, { color: tokens.ink }]} numberOfLines={1}>
            {agent.agent}
          </Text>
          {agent.model ? (
            <Text style={[typeScale.monoMeta, styles.model, { color: tokens.ink4 }]} numberOfLines={1}>
              {agent.model}
            </Text>
          ) : null}
          {!failed ? (
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]}>{formatCost(agent.cost)}</Text>
          ) : null}
        </View>
        {agent.last ? (
          <Text
            style={[styles.tail, { color: failed ? tokens.danger : tokens.ink3 }]}
            numberOfLines={2}
          >
            {agent.last}
          </Text>
        ) : null}
      </View>
      {showSeparator && !selected ? (
        <View style={[styles.agentSeparator, { backgroundColor: tokens.hairline }]} />
      ) : null}
    </Pressable>
  );
}

function PhaseBlock({
  group,
  index,
  selectedId,
  onSelectAgent,
}: {
  group: PhaseGroup;
  index: number;
  selectedId: string | null;
  onSelectAgent: (agent: SnapshotSubagent) => void;
}) {
  const tokens = useTokens();
  const entrance = useForgeline(index);
  const running = group.state === "running";
  const label = group.unknown ? "unphased" : group.phase;

  return (
    <Animated.View style={[styles.block, entrance]}>
      {running ? <HeatEdge state="busy" /> : null}
      <View style={styles.phaseHeader}>
        <Medallion state={group.state} />
        <Text
          style={[typeScale.bodyBold, styles.phaseName, { color: group.state === "pending" ? tokens.ink3 : tokens.ink }]}
          numberOfLines={1}
        >
          {label}
        </Text>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          {phaseSublabel(group)}
        </Text>
      </View>
      {group.rows.length > 0 ? (
        <View style={styles.agents}>
          {group.rows.map((agent, rowIndex) => (
            <AgentLiveRow
              key={agent.id}
              agent={agent}
              selected={agent.id === selectedId}
              showSeparator={rowIndex < group.rows.length - 1}
              onPress={() => onSelectAgent(agent)}
            />
          ))}
        </View>
      ) : null}
    </Animated.View>
  );
}

export function PhaseTimeline({
  groups,
  selectedId,
  onSelectAgent,
}: {
  groups: PhaseGroup[];
  selectedId: string | null;
  onSelectAgent: (agent: SnapshotSubagent) => void;
}) {
  return (
    <View>
      {groups.map((group, index) => (
        <PhaseBlock
          key={group.phase}
          group={group}
          index={index}
          selectedId={selectedId}
          onSelectAgent={onSelectAgent}
        />
      ))}
    </View>
  );
}

const GUTTER = 8;
const NEST = 30;

const styles = StyleSheet.create({
  block: { position: "relative", paddingLeft: GUTTER, marginTop: space.space16 },
  phaseHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  medallion: { width: 20, height: 20, borderRadius: 10, alignItems: "center", justifyContent: "center" },
  medallionPending: { borderWidth: 2, backgroundColor: "transparent" },
  phaseName: { flex: 1, fontSize: 15.5 },
  agents: { marginLeft: NEST - GUTTER, marginTop: space.space4 },
  agentWrap: { position: "relative", paddingHorizontal: space.space8 },
  agentRow: { paddingVertical: space.space8, gap: space.space4 },
  agentHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  agentName: { flex: 1, fontSize: 14, fontWeight: "600" },
  model: { flexShrink: 0 },
  tail: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 16, marginLeft: 15 },
  agentSeparator: { height: StyleSheet.hairlineWidth, marginLeft: 15 },
  dotWrap: { width: 8, height: 8, alignItems: "center", justifyContent: "center" },
  dot: { width: 7, height: 7, borderRadius: 3.5 },
  dotRing: { position: "absolute", width: 13, height: 13, borderRadius: 6.5, borderWidth: 1.5 },
});
