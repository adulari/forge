// A run of tool calls folded into one dim row (lib/toolGroups.ts, after the TUI's tool groups):
// "Read 3 files, ran 2 shell commands", with "1 failed" on the right where it is never cut. Tap
// or click (Enter/Space on desktop and web) opens it in place to the per-call cards, unchanged.
// Collapsed, a failed call and the call in flight stay listed under the row: a failure is the
// one thing a reader needs without asking. Permission prompts live in CardSlot above the
// composer, so folding never hides a decision.
import { ChevronDown, ChevronRight } from "lucide-react-native";
import React, { useCallback } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import { collapsedCalls, type ToolGroupEntry } from "../../lib/toolGroups";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { ToolCallRow } from "./ToolCallRow";

export interface ToolGroupRowProps {
  group: ToolGroupEntry;
  expanded: boolean;
  onToggle: (key: string) => void;
}

export function groupAccessibilityLabel(group: ToolGroupEntry): string {
  const { failed, running } = group.summary;
  const parts = [group.text || "Tool calls"];
  if (failed > 0) parts.push(`${failed} failed`);
  if (running > 0) parts.push(`${running} running`);
  return parts.join(", ");
}

function ToolGroupRowImpl({ group, expanded, onToggle }: ToolGroupRowProps) {
  const tokens = useTokens();
  const { summary, key } = group;
  const toggle = useCallback(() => {
    haptics.select();
    onToggle(key);
  }, [key, onToggle]);
  // The first call of a group shows alone until it finishes: there is nothing to sum up yet.
  const hasSummary = summary.finished > 0;
  const running = summary.running > 0;
  const calls = !hasSummary || expanded ? group.calls : collapsedCalls(group);

  return (
    <View>
      {hasSummary ? (
        <Pressable
          onPress={toggle}
          accessibilityRole="button"
          accessibilityLabel={groupAccessibilityLabel(group)}
          accessibilityHint={expanded ? "Hides the individual tool calls" : "Shows each tool call"}
          accessibilityState={{ expanded }}
          focusable
          style={({ pressed }) => [styles.summary, pressed ? { backgroundColor: tokens.bg2 } : null]}
          hitSlop={4}
        >
          <View style={styles.mark}>
            {running && !expanded ? (
              <ActivityIndicator size="small" color={tokens.ink3} style={styles.spinner} />
            ) : expanded ? (
              <ChevronDown size={14} strokeWidth={2} color={tokens.ink3} />
            ) : (
              <ChevronRight size={14} strokeWidth={2} color={tokens.ink3} />
            )}
          </View>
          <Text style={[type.meta, styles.text, { color: tokens.ink3 }]} numberOfLines={2}>
            {group.text || "Working"}
          </Text>
          {summary.failed > 0 ? (
            <Text style={[type.meta, styles.failed, { color: tokens.danger }]}>{summary.failed} failed</Text>
          ) : null}
        </Pressable>
      ) : null}
      {calls.length > 0 ? (
        <View style={hasSummary ? styles.nested : undefined}>
          {calls.map((call) => (
            <ToolCallRow key={call.key} invocation={call.invocation} />
          ))}
        </View>
      ) : null}
    </View>
  );
}

export const ToolGroupRow = React.memo(ToolGroupRowImpl);

const styles = StyleSheet.create({
  // Same gutter as ToolCallRow, so a group sits where its first card would.
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    marginLeft: space.space24,
    marginRight: space.space16,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    minHeight: 32,
  },
  mark: { width: 14, height: 14, alignItems: "center", justifyContent: "center" },
  spinner: { transform: [{ scale: 0.6 }] },
  text: { flex: 1, minWidth: 0 },
  failed: { flexShrink: 0 },
  nested: { paddingLeft: space.space16 },
});
