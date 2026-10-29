// The task plan above the composer while a session works through it: done rows dimmed, the one
// in progress with a pulsing dot, the rest as hollow circles. Fed by `snapshot.tasks`, so it
// follows `update_tasks` live. Folding it leaves the header and the headline task.
import { Check, ListChecks } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { summarizeTasks, windowTasks } from "../../lib/taskDock";
import type { SnapshotTask } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import type { MenuEntry } from "../shell/PopoverMenu";
import { DockFrame } from "./DockFrame";

const MAX_ROWS = 6;

function Glyph({ status, busy }: { status: SnapshotTask["status"]; busy: boolean }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(status === "in_progress" && busy ? "busy" : "idle");
  if (status === "done") return <Check size={12} strokeWidth={2.25} color={tokens.ink4} />;
  if (status === "in_progress") return <Animated.View style={[styles.dot, { backgroundColor: tokens.accent }, dotStyle]} />;
  return <View style={[styles.ring, { borderColor: tokens.ink4 }]} />;
}

function Row({ task, busy }: { task: SnapshotTask; busy: boolean }) {
  const tokens = useTokens();
  const done = task.status === "done";
  const active = task.status === "in_progress";
  return (
    <View
      style={styles.row}
      accessibilityRole="text"
      accessibilityLabel={`${task.title}, ${task.status === "in_progress" ? "in progress" : task.status}`}
    >
      <View style={styles.glyph}>
        <Glyph status={task.status} busy={busy} />
      </View>
      <Text
        style={[typeScale.sub, styles.title, { color: done ? tokens.ink4 : active ? tokens.ink : tokens.ink2 }]}
        numberOfLines={1}
      >
        {task.title}
      </Text>
    </View>
  );
}

export interface TaskDockProps {
  tasks: readonly SnapshotTask[];
  busy: boolean;
  collapsed: boolean;
  onToggle: () => void;
  menu: readonly MenuEntry[];
}

export function TaskDock({ tasks, busy, collapsed, onToggle, menu }: TaskDockProps) {
  const tokens = useTokens();
  if (tasks.length === 0) return null;
  const summary = summarizeTasks(tasks);
  const window = windowTasks(tasks, MAX_ROWS);
  return (
    <DockFrame
      title="Plan"
      icon={ListChecks}
      meta={`${summary.done}/${summary.total}`}
      headline={summary.allDone ? "all done" : summary.headline?.title}
      collapsed={collapsed}
      onToggle={onToggle}
      menu={menu}
      testID="task-dock"
    >
      <View style={styles.body}>
        {window.hiddenAbove > 0 ? (
          <Text style={[typeScale.meta, styles.more, { color: tokens.ink4 }]}>{`${window.hiddenAbove} earlier`}</Text>
        ) : null}
        {window.rows.map(({ index, task }) => (
          <Row key={`${index}-${task.title}`} task={task} busy={busy} />
        ))}
        {window.hiddenBelow > 0 ? (
          <Text style={[typeScale.meta, styles.more, { color: tokens.ink4 }]}>{`${window.hiddenBelow} more`}</Text>
        ) : null}
      </View>
    </DockFrame>
  );
}

const styles = StyleSheet.create({
  body: { paddingBottom: space.space4 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 24, paddingLeft: space.space12, paddingRight: space.space8 },
  glyph: { width: 14, alignItems: "center", justifyContent: "center" },
  title: { flex: 1, minWidth: 0 },
  dot: { width: 6, height: 6, borderRadius: 3 },
  ring: { width: 9, height: 9, borderRadius: 4.5, borderWidth: 1.5 },
  more: { paddingLeft: space.space12 + 14 + space.space8, paddingVertical: 1 },
});
