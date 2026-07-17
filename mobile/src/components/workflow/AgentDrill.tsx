// Workflow agent drill-in. The wire carries no per-agent transcript and no retry/skip verb,
// so this shows only what's real: task, a model·phase·cost meta line, the live tail, and —
// once the agent is done and its last line embeds JSON — a structured-output block. The only
// control is Interrupt, which stops the WHOLE run (there is no per-agent stop on the wire).
import { ArrowLeft } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { SnapshotSubagent } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { HeatEdge } from "../ds/HeatEdge";
import { extractJson, isFailed } from "./format";
import { StructuredOutput } from "./StructuredOutput";

function DrillDot({ agent }: { agent: SnapshotSubagent }) {
  const tokens = useTokens();
  const kind = isFailed(agent) ? "waiting" : agent.done ? "idle" : "busy";
  const { dotStyle } = useEmberdot(kind);
  const color = kind === "waiting" ? tokens.danger : kind === "busy" ? tokens.accent : tokens.ink3;
  return <Animated.View style={[styles.dot, { backgroundColor: color }, dotStyle]} />;
}

function SectionLabel({ children }: { children: string }) {
  const tokens = useTokens();
  return <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>{children}</Text>;
}

export function AgentDrill({
  agent,
  active,
  onBack,
  onStop,
}: {
  agent: SnapshotSubagent | null;
  active: boolean;
  onBack?: () => void;
  onStop?: () => void;
}) {
  const tokens = useTokens();

  if (!agent) {
    return (
      <View style={styles.empty}>
        <Text style={[typeScale.sub, { color: tokens.ink3, textAlign: "center" }]}>
          Select an agent to inspect its task and result.
        </Text>
      </View>
    );
  }

  const failed = isFailed(agent);
  const meta = [agent.model ?? "—", `phase ${agent.phase ?? "—"}`, formatCost(agent.cost)].join(" · ");
  const resultJson = agent.done ? extractJson(agent.last) : null;

  return (
    <View style={styles.wrap}>
      <View style={styles.titleRow}>
        {onBack ? (
          <Pressable onPress={onBack} accessibilityRole="button" accessibilityLabel="Back to timeline" hitSlop={6} style={styles.back}>
            <ArrowLeft size={20} strokeWidth={2} color={tokens.ink2} />
          </Pressable>
        ) : null}
        <DrillDot agent={agent} />
        <Text style={[typeScale.headingBold, styles.name, { color: tokens.ink }]} numberOfLines={1}>
          {agent.agent}
        </Text>
        <Text style={[styles.badge, { color: tokens.ink2, borderColor: tokens.border }]}>read-only</Text>
      </View>
      <Text style={[typeScale.monoMeta, tabularNums, styles.meta, { color: tokens.ink3 }]} numberOfLines={1}>
        {meta}
      </Text>

      <View style={[styles.divider, { backgroundColor: tokens.border }]} />

      {agent.task ? (
        <>
          <SectionLabel>task · from workflow</SectionLabel>
          <Text style={[typeScale.body, styles.task, { color: tokens.ink2 }]}>{agent.task}</Text>
        </>
      ) : null}

      {resultJson != null ? (
        <>
          <SectionLabel>structured output</SectionLabel>
          <StructuredOutput data={resultJson} label="result" />
        </>
      ) : agent.last ? (
        <>
          <SectionLabel>{agent.done ? "result" : "live"}</SectionLabel>
          <View style={[styles.tailBox, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
            {failed ? <HeatEdge state="waiting" /> : null}
            <Text style={[styles.tail, { color: failed ? tokens.danger : tokens.ink2 }]}>{agent.last}</Text>
          </View>
        </>
      ) : null}

      {active && onStop ? (
        <Pressable
          onPress={onStop}
          accessibilityRole="button"
          accessibilityLabel="Interrupt workflow"
          style={[styles.interrupt, { borderColor: tokens.borderStrong }]}
        >
          <Text style={[typeScale.bodyBold, { color: tokens.danger }]}>Interrupt</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space8 },
  empty: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space32 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  back: { width: 28, height: 44, marginLeft: -6, alignItems: "center", justifyContent: "center" },
  dot: { width: 8, height: 8, borderRadius: 4 },
  name: { flex: 1 },
  badge: {
    ...typeScale.monoMeta,
    fontSize: 10.5,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    paddingHorizontal: 6,
    paddingVertical: 1,
    overflow: "hidden",
  },
  meta: { marginLeft: 16 },
  divider: { height: StyleSheet.hairlineWidth, marginVertical: space.space4 },
  sectionLabel: { marginTop: space.space12 },
  task: { marginTop: space.space4 },
  tailBox: {
    position: "relative",
    marginTop: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius12,
    padding: space.space12,
    overflow: "hidden",
  },
  tail: { fontFamily: monoFamily.regular, fontSize: 12, lineHeight: 18 },
  interrupt: {
    alignSelf: "flex-start",
    minHeight: tapTarget,
    justifyContent: "center",
    paddingHorizontal: space.space16,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius12,
    marginTop: space.space16,
  },
});
