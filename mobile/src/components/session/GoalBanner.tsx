// Native Features pack — "NF Goal" / "NF Desktop Goal" banner-card. Hearth pattern:
// an ember decision card pinned under the session header (compact) / at the top of the
// chat column (medium+) while an autonomous `/goal` loop is running. Dash-marked "Goal"
// header, the goal condition, an active emberdot, and a Stop control.
//
// HONESTY (live-data contract): the daemon exposes NO structured goal state on the
// snapshot — no condition field, no iteration counter, no judge verdicts. The only
// signal on the wire is the transient `notes` line the CLI emits when `/goal` runs
// ("🎯 goal set — <condition>", "🎯 goal started", "🎯 goal stopped"). This banner
// derives the active goal + its condition from those notes and renders nothing when no
// active goal is detectable. The prototype's iteration/cost/judge chips are NOT on the
// wire, so they are omitted (see report). Stop maps to `{kind:"interrupt"}` — the CLI
// breaks a running goal loop on interrupt (run.rs: "a `/goal` in progress stops on
// interrupt"); there is no `/goal stop` verb (that would set a goal literally named
// "stop").
import { Square } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import type { RemoteInput, Snapshot } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Card } from "../ds/Card";
import { StatusDot } from "../ds/StatusDot";

export interface GoalBannerProps {
  snapshot: Snapshot | null;
  send: (input: RemoteInput) => boolean;
}

/** Derive `{ condition }` for an active goal from the transient CLI notes, or null. */
export function deriveActiveGoal(notes: string[] | undefined): { condition: string } | null {
  if (!notes || notes.length === 0) return null;
  let condition: string | null = null;
  let active = false;
  for (const note of notes) {
    const lower = note.toLowerCase();
    if (lower.includes("goal set")) {
      const match = note.match(/goal set\s*[—–-]?\s*(.*)$/i);
      const text = match?.[1]?.trim();
      if (text) condition = text;
      active = true;
    } else if (lower.includes("goal started")) {
      active = true;
    } else if (lower.includes("goal stopped") || lower.includes("goal complete")) {
      active = false;
    }
  }
  if (!active) return null;
  return { condition: condition ?? "Autonomous goal running" };
}

export function GoalBanner({ snapshot, send }: GoalBannerProps) {
  const tokens = useTokens();
  const goal = deriveActiveGoal(snapshot?.notes);
  if (!goal) return null;

  const stop = () => {
    haptics.deny();
    send({ kind: "interrupt" });
  };

  return (
    <View style={styles.wrap}>
      <Card heatEdge="busy" style={styles.card}>
        <View style={styles.header}>
          <StatusDot state="busy" size={6} accessibilityLabel="goal active" />
          <View style={[styles.dash, { backgroundColor: tokens.accent }]} />
          <Text style={[typeScale.section, { color: tokens.accent }]}>Goal · active</Text>
          <View style={styles.spacer} />
          <Pressable
            onPress={stop}
            accessibilityRole="button"
            accessibilityLabel="Stop goal"
            hitSlop={8}
            style={styles.stop}
          >
            <Square size={11} color={tokens.danger} fill={tokens.danger} strokeWidth={0} />
            <Text style={[typeScale.meta, { color: tokens.danger }]}>Stop</Text>
          </Pressable>
        </View>
        <Text style={[typeScale.body, styles.condition, { color: tokens.ink }]}>{goal.condition}</Text>
      </Card>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { paddingBottom: space.space8 },
  card: { gap: space.space8 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  spacer: { flex: 1 },
  stop: { flexDirection: "row", alignItems: "center", gap: space.space4, minHeight: 28, paddingLeft: space.space8 },
  condition: { marginTop: space.space2 },
});
