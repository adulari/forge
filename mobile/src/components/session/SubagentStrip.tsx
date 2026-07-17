// Hearth in-chat subagents strip: the collapsed inline appearance of a `spawn_agents`
// batch, designed to sit just above the composer. Shows a bot glyph + count + a mono
// running/failed/cost summary, then emberdot pills for the RUNNING children (capped, with
// a "+N more" collapse). Renders nothing unless a phase-less child is running. `onPress`
// (optional) opens the full Subagents panel. Exported for the integration agent to mount.
import { Bot } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { SnapshotSubagent } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

const MAX_PILLS = 3;

function Pill({ name }: { name: string }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot("busy");
  return (
    <View style={[styles.pill, { backgroundColor: tokens.bg3 }]}>
      <View style={styles.pillDotWrap}>
        <Animated.View style={[styles.pillDot, { backgroundColor: tokens.accent }, dotStyle]} />
      </View>
      <Text style={[styles.pillText, { color: tokens.ink2 }]} numberOfLines={1}>
        {name}
      </Text>
    </View>
  );
}

export interface SubagentStripProps {
  subagents: SnapshotSubagent[];
  /** Optional — opens the full Subagents panel. */
  onPress?: () => void;
}

function SubagentStripBase({ subagents, onPress }: SubagentStripProps) {
  const tokens = useTokens();
  const rows = subagents.filter((s) => s.phase == null);
  const running = rows.filter((s) => !s.done);
  if (running.length === 0) return null;

  const failed = rows.filter((s) => s.done && !s.ok).length;
  const totalCost = rows.reduce((sum, s) => sum + s.cost, 0);
  const shown = running.slice(0, MAX_PILLS);
  const overflow = running.length - shown.length;
  const meta = [`${running.length} running`, failed > 0 ? `${failed} failed` : null].filter(Boolean).join(" · ");

  const content = (
    <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={styles.head}>
        <Bot size={13} strokeWidth={1.75} color={tokens.accent} />
        <Text style={[styles.title, typeScale.meta, { color: tokens.ink2 }]} numberOfLines={1}>
          {rows.length} {rows.length === 1 ? "subagent" : "subagents"}
        </Text>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>
          {`${meta} · `}
          <Text style={{ color: tokens.success }}>{formatCost(totalCost)}</Text>
        </Text>
      </View>
      <View style={styles.pills}>
        {shown.map((s) => (
          <Pill key={s.id} name={s.agent} />
        ))}
        {overflow > 0 ? (
          <View style={[styles.pill, { backgroundColor: tokens.bg3 }]}>
            <Text style={[styles.pillText, { color: tokens.ink3 }]}>+{overflow} more</Text>
          </View>
        ) : null}
      </View>
    </View>
  );

  if (!onPress) return content;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${running.length} running subagents — open panel`}
    >
      {content}
    </Pressable>
  );
}

export const SubagentStrip = React.memo(SubagentStripBase);

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: 14, paddingHorizontal: 13, paddingVertical: 11 },
  head: { flexDirection: "row", alignItems: "center", gap: 8 },
  title: { flex: 1, fontWeight: "600", fontSize: 12.5 },
  pills: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: 6, marginTop: 8 },
  pill: {
    height: 22,
    paddingHorizontal: 9,
    borderRadius: 999,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
  },
  pillDotWrap: { width: 5, height: 5, alignItems: "center", justifyContent: "center" },
  pillDot: { width: 5, height: 5, borderRadius: 2.5 },
  pillText: { fontFamily: monoFamily.regular, fontSize: 10, lineHeight: 14 },
});
