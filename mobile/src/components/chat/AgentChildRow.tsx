// One child agent a delegating call (`spawn_agents`, a workflow) started, as its own row under
// that call: status, who, what it was asked, which model, what it cost. Tapping opens the child's
// own transcript. Rows come from the host's agent index (lib/agentLinks.tsx).
import { Check, ChevronRight, X } from "lucide-react-native";
import React from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import type { AgentIndexRow } from "../../lib/api";
import { haptics } from "../../lib/haptics";
import { openSession } from "../../lib/nav";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type } from "../../theme/typography";
import { shortModelLabel } from "../fleet/shortModelLabel";

export function agentStatusLabel(status: AgentIndexRow["status"]): string {
  switch (status) {
    case "running":
      return "running";
    case "done":
      return "done";
    case "failed":
      return "failed";
    default:
      return "cancelled";
  }
}

function AgentChildRowImpl({ child }: { child: AgentIndexRow }) {
  const tokens = useTokens();
  const status = agentStatusLabel(child.status);
  const bad = child.status === "failed" || child.status === "cancelled";
  return (
    <Pressable
      onPress={() => {
        haptics.select();
        openSession(child.session_id);
      }}
      accessibilityRole="button"
      accessibilityLabel={`Open ${child.agent} transcript, ${status}${child.task ? `, ${child.task}` : ""}`}
      style={({ pressed }) => [styles.row, pressed ? { backgroundColor: tokens.bg2 } : null]}
      hitSlop={4}
    >
      <View style={styles.mark}>
        {child.status === "running" ? (
          <ActivityIndicator size="small" color={tokens.accent} style={styles.spinner} />
        ) : bad ? (
          <X size={13} strokeWidth={2} color={tokens.danger} />
        ) : (
          <Check size={13} strokeWidth={2} color={tokens.success} />
        )}
      </View>
      <View style={styles.body}>
        <View style={styles.head}>
          <Text style={[type.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
            {child.agent}
          </Text>
          {child.model ? (
            <View style={[styles.chip, { backgroundColor: tokens.bg3 }]}>
              <Text style={[styles.chipText, { color: tokens.ink3 }]} numberOfLines={1}>
                {shortModelLabel(child.model)}
              </Text>
            </View>
          ) : null}
          <View style={styles.spacer} />
          <Text style={[type.meta, tabularNums, { color: bad ? tokens.danger : tokens.ink4 }]}>
            {bad || child.status === "running" ? status : child.cost_usd > 0 ? formatCost(child.cost_usd) : status}
          </Text>
        </View>
        {child.task ? (
          <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={2}>
            {child.task}
          </Text>
        ) : null}
      </View>
      <ChevronRight size={13} strokeWidth={1.75} color={tokens.ink4} />
    </Pressable>
  );
}

export const AgentChildRow = React.memo(AgentChildRowImpl);

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "flex-start", gap: space.space8, paddingHorizontal: space.space8, paddingVertical: space.space4, borderRadius: radii.radius8, minHeight: 32 },
  mark: { width: 14, height: 18, alignItems: "center", justifyContent: "center" },
  spinner: { transform: [{ scale: 0.6 }] },
  body: { flex: 1, minWidth: 0, gap: 1 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  spacer: { flex: 1 },
  chip: { flexShrink: 1, maxWidth: 160, height: 18, paddingHorizontal: space.space8, justifyContent: "center", borderRadius: radii.radiusPill },
  chipText: { fontFamily: monoFamily.regular, fontSize: 10, lineHeight: 14 },
});
