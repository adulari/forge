// Tasks segment (BUILD_PLAN §6 "Tasks" / §7 Batch 3 W7). Renders `snapshot.tasks` live from
// the shared session socket (no fetch — sessionContext already owns the one WS connection).
// Mirrors remote_assets/app.js's `renderTasks` glyph scheme (○ pending / ◐ in_progress /
// ● done, done rows dimmed + struck-through) — see remote_assets/styles.css `.task`.
import React, { useCallback, useMemo } from "react";
import { Text, View } from "react-native";

import { BoundedList, EmptyState, Loading } from "../../../components/ui";
import { useSessionCtx } from "../../../lib/sessionContext";
import type { SnapshotTask } from "../../../lib/ws";

const STATUS_GLYPH: Record<SnapshotTask["status"], string> = {
  pending: "○",
  in_progress: "◐",
  done: "●",
};

const STATUS_GLYPH_CLASS: Record<SnapshotTask["status"], string> = {
  pending: "text-dim",
  in_progress: "text-accent",
  done: "text-ok",
};

interface TaskRow extends SnapshotTask {
  id: string;
}

function TaskRowItemBase({ item }: { item: TaskRow }) {
  const isDone = item.status === "done";
  return (
    <View className="flex-row items-baseline gap-8 px-10 py-8 border-b border-histBorder">
      <Text className={STATUS_GLYPH_CLASS[item.status]} style={{ fontSize: 15 }}>
        {STATUS_GLYPH[item.status]}
      </Text>
      <Text
        numberOfLines={2}
        className={
          isDone ? "flex-1 text-dim text-[14px] line-through" : "flex-1 text-ink text-[14px]"
        }
      >
        {item.title}
      </Text>
    </View>
  );
}

const TaskRowItem = React.memo(TaskRowItemBase, (prev, next) => {
  const a = prev.item;
  const b = next.item;
  return a.id === b.id && a.status === b.status && a.title === b.title;
});

export default function TasksScreen() {
  const { snapshot } = useSessionCtx();

  const data = useMemo<TaskRow[]>(
    () => (snapshot?.tasks ?? []).map((t, i) => ({ ...t, id: `${i}:${t.title}` })),
    [snapshot?.tasks],
  );

  const keyExtractor = useCallback((item: TaskRow) => item.id, []);
  const renderItem = useCallback(
    ({ item }: { item: TaskRow }) => <TaskRowItem item={item} />,
    [],
  );
  const emptyComponent = useMemo(() => <EmptyState title="No tasks yet." />, []);

  if (!snapshot) {
    return (
      <View className="flex-1">
        <Loading label="Connecting to session…" />
      </View>
    );
  }

  const done = data.filter((t) => t.status === "done").length;

  return (
    <View className="flex-1">
      {data.length ? (
        <Text
          className="text-dim text-[12px] font-semibold uppercase tracking-[0.5px] mb-6"
          style={{ fontVariant: ["tabular-nums"] }}
        >
          {done}/{data.length} done
        </Text>
      ) : null}
      <BoundedList
        data={data}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
      />
    </View>
  );
}
