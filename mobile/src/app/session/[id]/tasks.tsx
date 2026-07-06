// T3.4 — Tasks segment: read-only `snapshot.tasks` rows (FEATURES.md §1.2 `tasks` -> Tasks
// segment). Per T3.1 HANDOFF this segment owns its own Screen (edges omit "top" — the shell's
// header/status-strip/Segmented already consumed the top inset).
import { ListChecks } from "lucide-react-native";
import React, { useCallback } from "react";

import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { TaskRow } from "../../../components/fleet/TaskRow";
import { useSessionCtx } from "../../../lib/sessionContext";
import type { SnapshotTask } from "../../../lib/ws";

export default function SessionTasks() {
  const { snapshot } = useSessionCtx();
  const tasks = snapshot?.tasks ?? [];
  const busy = snapshot?.busy ?? false;

  const renderItem = useCallback(
    ({ item }: { item: SnapshotTask }) => <TaskRow task={item} busy={busy} />,
    [busy],
  );
  // Snapshot.tasks has no stable id — index+title is stable enough for this read-only,
  // effectively-append-only list.
  const keyExtractor = useCallback((item: SnapshotTask, index: number) => `${index}-${item.title}`, []);

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false}>
      <BoundedList
        data={tasks}
        renderItem={renderItem}
        keyExtractor={keyExtractor}
        ListEmptyComponent={<EmptyState icon={ListChecks} message="no tasks yet" />}
      />
    </Screen>
  );
}
