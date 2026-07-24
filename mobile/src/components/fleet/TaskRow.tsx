// Machined `TaskRow` (Mobile/Desktop "Session Tasks" frames): dense hairline-separated
// rows — done = check + strikethrough dim title + "done" mono tag, in_progress = pulsing
// filled accent dot + a neutral row-highlight wash + mono status tag (assignee when the
// caller has one, else "in progress" — `SnapshotTask` carries no assignee field on the
// wire today, see `assignee` prop doc), pending = hollow ring + "queued" mono tag.
import { Check } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, rowHeight, space, type ColorTokens } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import type { SnapshotTask } from "../../lib/ws";

const GLYPH_SIZE = 15;

export interface TaskRowProps {
  task: SnapshotTask;
  /** Session-level `busy` (Snapshot.busy) — gates the in_progress dot's pulse ("live" heat). */
  busy?: boolean;
  showSeparator?: boolean;
  /** Who's working this task, mono-tagged beside "in progress" — omitted when the caller
   * doesn't have one (the wire's `SnapshotTask` carries no assignee field today). */
  assignee?: string;
}

function TaskGlyph({ status, busy }: { status: SnapshotTask["status"]; busy: boolean }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(status === "in_progress" && busy ? "busy" : "idle");

  if (status === "done") {
    return <Check size={13} strokeWidth={2.5} color={tokens.ink3} />;
  }
  if (status === "in_progress") {
    return (
      <View style={styles.slot}>
        <Animated.View style={[styles.dot, { backgroundColor: tokens.accent }, dotStyle]} />
      </View>
    );
  }
  return (
    <View style={styles.slot}>
      <View style={[styles.ring, { borderColor: tokens.ink4 }]} />
    </View>
  );
}

function statusTag(task: SnapshotTask, assignee: string | undefined, tokens: ColorTokens) {
  if (task.status === "done") return { label: "done", color: tokens.ink3 };
  if (task.status === "in_progress") return { label: assignee ?? "in progress", color: tokens.accent };
  return { label: "queued", color: tokens.ink3 };
}

function TaskRowBase({ task, busy = false, showSeparator = true, assignee }: TaskRowProps) {
  const tokens = useTokens();
  const done = task.status === "done";
  const inProgress = task.status === "in_progress";
  const statusLabel = task.status === "in_progress" ? "in progress" : task.status;
  const tag = statusTag(task, assignee, tokens);

  return (
    <View>
      <View
        style={[
          styles.row,
          inProgress ? { backgroundColor: hexToRgba(tokens.ink, 0.05) } : null,
        ]}
        accessibilityRole="text"
        accessibilityLabel={`${task.title}, ${statusLabel}`}
      >
        <View style={styles.glyphSlot}>
          <TaskGlyph status={task.status} busy={busy} />
        </View>
        <Text
          style={[
            typeScale.body,
            styles.title,
            done
              ? { color: tokens.ink3, textDecorationLine: "line-through" }
              : { color: tokens.ink },
          ]}
          numberOfLines={2}
        >
          {task.title}
        </Text>
        <Text style={[typeScale.monoMeta, tabularNums, styles.tag, { color: tag.color }]} numberOfLines={1}>
          {tag.label}
        </Text>
      </View>
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

export const TaskRow = React.memo(TaskRowBase);

const styles = StyleSheet.create({
  row: {
    minHeight: rowHeight.dense,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space16,
    gap: space.space12,
    borderRadius: 3,
  },
  glyphSlot: { width: GLYPH_SIZE, alignItems: "center", justifyContent: "center" },
  title: { flex: 1 },
  tag: { flexShrink: 0 },
  slot: { alignItems: "center", justifyContent: "center" },
  dot: { width: 6, height: 6, borderRadius: 3 },
  ring: { width: 9, height: 9, borderRadius: 4.5, borderWidth: 1.5 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
});
