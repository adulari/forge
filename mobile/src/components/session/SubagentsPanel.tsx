// Hearth Subagents viewer for plain `spawn_agents` batches (subagents whose `phase == null`).
// Responsive:
//   compact  — de-boxed hairline rows (AgentRow), tap to expand inline detail.
//   medium+  — a 2/3-column tile grid (bg2 cards, radius16) per the desktop prototype.
// Running rows carry the accent heat edge + live tail; failed (done && !ok) get danger
// treatment; settled (done && ok) dim with an ink4 check. Empty state per the prototype.
import { Bot } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { type LayoutChangeEvent, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { SnapshotSubagent } from "../../lib/ws";
import { useForgeline } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { EmptyState } from "../ds/EmptyState";
import { HeatEdge } from "../ds/HeatEdge";
import { StatusDot } from "../ds/StatusDot";
import { AgentRow, rowStateOf } from "./AgentRow";

const GRID_GAP = 20;
const TAIL_RADIUS = 9;

export interface SubagentsPanelProps {
  subagents: SnapshotSubagent[];
}

function CompactRow({
  agent,
  index,
  expanded,
  onToggle,
  showSeparator,
}: {
  agent: SnapshotSubagent;
  index: number;
  expanded: boolean;
  onToggle: () => void;
  showSeparator: boolean;
}) {
  const entrance = useForgeline(index);
  return (
    <Animated.View style={entrance}>
      <AgentRow agent={agent} expanded={expanded} onPress={onToggle} showSeparator={showSeparator} />
    </Animated.View>
  );
}

function Tile({
  agent,
  index,
  width,
  expanded,
  onToggle,
}: {
  agent: SnapshotSubagent;
  index: number;
  width: number | undefined;
  expanded: boolean;
  onToggle: () => void;
}) {
  const tokens = useTokens();
  const entrance = useForgeline(index);
  const state = rowStateOf(agent);
  const running = state === "running";
  const failed = state === "failed";
  const done = state === "done";

  return (
    <Animated.View style={[{ width: width ?? "100%" }, entrance]}>
      <Pressable
        onPress={onToggle}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        accessibilityLabel={`Subagent ${agent.agent}`}
        style={[
          styles.tile,
          { backgroundColor: tokens.bg2, borderColor: tokens.border },
          done && styles.dimmed,
        ]}
      >
        {running ? <HeatEdge state="busy" /> : failed ? <HeatEdge state="waiting" /> : null}
        <View style={styles.tileInner}>
          <View style={styles.header}>
            {running ? (
              <StatusDot state="busy" />
            ) : failed ? (
              <View style={[styles.failDot, { backgroundColor: tokens.danger }]} />
            ) : (
              <StatusDot state="done" />
            )}
            <Text
              style={[typeScale.bodyBold, styles.name, { color: done ? tokens.ink2 : tokens.ink }]}
              numberOfLines={1}
            >
              {agent.agent}
            </Text>
            {agent.model ? (
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                {agent.model}
              </Text>
            ) : null}
            <Text style={[typeScale.monoMeta, tabularNums, { color: failed ? tokens.ink3 : tokens.success }]}>
              {formatCost(agent.cost)}
            </Text>
          </View>
          {agent.task ? (
            <Text style={[typeScale.sub, styles.tileTask, { color: tokens.ink3 }]} numberOfLines={expanded ? undefined : 1}>
              {agent.task}
            </Text>
          ) : null}
          {agent.last ? (
            running ? (
              <View style={[styles.tailBlock, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
                <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={expanded ? undefined : 2}>
                  {agent.last}
                </Text>
              </View>
            ) : (
              <Text
                style={[typeScale.monoMeta, styles.tileTail, { color: failed ? tokens.danger : tokens.ink4 }]}
                numberOfLines={expanded ? undefined : 2}
              >
                {agent.last}
              </Text>
            )
          ) : null}
        </View>
      </Pressable>
    </Animated.View>
  );
}

function SubagentsPanelBase({ subagents }: SubagentsPanelProps) {
  const tokens = useTokens();
  const { isCompact, isExpanded } = useBreakpoint();
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [gridWidth, setGridWidth] = useState(0);

  const rows = subagents.filter((s) => s.phase == null);
  const running = rows.filter((s) => !s.done).length;
  const totalCost = rows.reduce((sum, s) => sum + s.cost, 0);

  const toggle = useCallback(
    (id: string) => setExpandedId((current) => (current === id ? null : id)),
    [],
  );

  const onGridLayout = useCallback((e: LayoutChangeEvent) => {
    setGridWidth(e.nativeEvent.layout.width);
  }, []);

  if (rows.length === 0) {
    return (
      <View style={[styles.container, isCompact ? styles.compactGutter : styles.wideGutter]}>
        <EmptyState icon={Bot} message="No subagents this turn — the model is working alone." />
      </View>
    );
  }

  const columns = isExpanded ? 3 : 2;
  const tileWidth = gridWidth > 0 ? Math.floor((gridWidth - GRID_GAP * (columns - 1)) / columns) : undefined;

  return (
    <View style={[styles.container, isCompact ? styles.compactGutter : styles.wideGutter]}>
      <View style={styles.head}>
        <Text style={[typeScale.title, { color: tokens.ink }]}>Subagents</Text>
        <Text style={[typeScale.monoMeta, tabularNums, styles.summary, { color: tokens.ink3 }]} numberOfLines={1}>
          {`spawn_agents · this turn · ${running} of ${rows.length} running · `}
          <Text style={{ color: tokens.success }}>{formatCost(totalCost)}</Text>
        </Text>
      </View>

      {isCompact ? (
        <View style={styles.list}>
          {rows.map((agent, index) => (
            <CompactRow
              key={agent.id}
              agent={agent}
              index={index}
              expanded={expandedId === agent.id}
              onToggle={() => toggle(agent.id)}
              showSeparator={index < rows.length - 1}
            />
          ))}
        </View>
      ) : (
        <View style={styles.grid} onLayout={onGridLayout}>
          {rows.map((agent, index) => (
            <Tile
              key={agent.id}
              agent={agent}
              index={index}
              width={tileWidth}
              expanded={expandedId === agent.id}
              onToggle={() => toggle(agent.id)}
            />
          ))}
        </View>
      )}
    </View>
  );
}

export const SubagentsPanel = React.memo(SubagentsPanelBase);

const styles = StyleSheet.create({
  container: { width: "100%" },
  compactGutter: { paddingHorizontal: space.space20, paddingTop: space.space12 },
  wideGutter: { maxWidth: 1100, alignSelf: "center", paddingHorizontal: space.space32, paddingTop: space.space24 },
  head: { gap: space.space4, marginBottom: space.space16 },
  summary: {},
  list: {},
  grid: { flexDirection: "row", flexWrap: "wrap", gap: GRID_GAP },
  tile: { position: "relative", borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, overflow: "hidden" },
  dimmed: { opacity: 0.7 },
  tileInner: { padding: space.space16, gap: space.space8 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1 },
  tileTask: { marginTop: -space.space4 },
  tailBlock: { borderWidth: StyleSheet.hairlineWidth, borderRadius: TAIL_RADIUS, paddingHorizontal: 10, paddingVertical: space.space8 },
  tileTail: {},
  failDot: { width: 8, height: 8, borderRadius: 4 },
});
