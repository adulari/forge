// Machined Subagents row (Mobile/Desktop "Session Agents" frames): a bordered card per
// child of a `spawn_agents` batch — running = bg2 card + dot + name + model·cost mono tag +
// live mono status line; needs-permission = stronger border + danger "needs permission" tag
// + inline Allow/Deny (only rendered when the caller supplies a prompt/handlers — the wire's
// `SnapshotSubagent` carries no per-agent permission field, only the session-level
// `Snapshot.permission_prompt`, so a caller must resolve which agent it belongs to); a
// settled child dims behind a faint border with a check + its last line as a diff-stat/cost
// caption.
import { Check } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { SnapshotSubagent } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { StatusDot } from "../ds/StatusDot";

export interface AgentRowProps {
  agent: SnapshotSubagent;
  showSeparator?: boolean;
  /** Inline detail open — unclamps the task + live tail (compact expandable detail). */
  expanded?: boolean;
  /** Makes the row a button that toggles its inline detail. Omit for a static row. */
  onPress?: () => void;
  /** Present only when the caller has resolved a live permission prompt to this specific
   * agent — renders the inline Allow/Deny bar. Omitted by default (see file header). */
  permissionPrompt?: string;
  onAllow?: () => void;
  onDeny?: () => void;
}

type RowState = "running" | "failed" | "done";

export function rowStateOf(agent: SnapshotSubagent): RowState {
  if (!agent.done) return "running";
  return agent.ok ? "done" : "failed";
}

function AgentRowBase({
  agent,
  showSeparator = true,
  expanded = false,
  onPress,
  permissionPrompt,
  onAllow,
  onDeny,
}: AgentRowProps) {
  const tokens = useTokens();
  const state = rowStateOf(agent);
  const running = state === "running";
  const failed = state === "failed";
  const done = state === "done";
  const needsPermission = running && permissionPrompt != null;

  const borderColor = needsPermission ? tokens.borderStrong : done ? tokens.hairline : tokens.border;
  const tailColor = failed ? tokens.danger : done ? tokens.ink4 : tokens.ink3;

  const body = (
    <View style={styles.body}>
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
        {needsPermission ? (
          <Text style={[typeScale.monoMeta, { color: tokens.danger }]} numberOfLines={1}>
            needs permission
          </Text>
        ) : (
          <>
            {agent.model ? (
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                {agent.model}
              </Text>
            ) : null}
            {done ? <Check size={14} strokeWidth={2} color={tokens.success} /> : null}
            <Text style={[typeScale.monoMeta, tabularNums, { color: failed ? tokens.ink3 : tokens.success }]}>
              {formatCost(agent.cost)}
            </Text>
          </>
        )}
      </View>

      {needsPermission ? (
        <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={expanded ? undefined : 2}>
          {permissionPrompt}
        </Text>
      ) : (
        <>
          {agent.task ? (
            <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={expanded ? undefined : 1}>
              {agent.task}
            </Text>
          ) : null}
          {agent.last ? (
            <Text
              style={[typeScale.monoMeta, styles.tail, { color: tailColor }]}
              numberOfLines={expanded ? undefined : 2}
            >
              {agent.last}
            </Text>
          ) : null}
        </>
      )}

      {needsPermission ? (
        <View style={styles.actions}>
          <Button label="Allow" variant="allow" onPress={onAllow ?? (() => {})} style={styles.allowBtn} />
          <Button label="Deny" variant="danger" onPress={onDeny ?? (() => {})} style={styles.denyBtn} />
        </View>
      ) : null}
    </View>
  );

  return (
    <View
      style={[
        styles.card,
        { backgroundColor: tokens.bg2, borderColor },
        showSeparator ? styles.cardSpacing : null,
        done ? styles.done : null,
      ]}
    >
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
    </View>
  );
}

export const AgentRow = React.memo(AgentRowBase);

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, overflow: "hidden" },
  cardSpacing: { marginBottom: space.space8 },
  done: { opacity: 0.85 },
  body: { paddingHorizontal: space.space12, paddingVertical: space.space12, gap: space.space4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  name: { flex: 1 },
  tail: { marginTop: space.space2 },
  failDot: { width: 8, height: 8, borderRadius: 4 },
  actions: { flexDirection: "row", gap: space.space8, marginTop: space.space4 },
  allowBtn: { flex: 1.4 },
  denyBtn: { flex: 1 },
});
