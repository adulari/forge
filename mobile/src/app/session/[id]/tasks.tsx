// T3.4 — Tasks segment: read-only `snapshot.tasks` rows (FEATURES.md §1.2 `tasks` -> Tasks
// segment). Per T3.1 HANDOFF this segment owns its own Screen (edges omit "top" — the shell's
// header/status-strip/Segmented already consumed the top inset).
import { ListChecks, WifiOff } from "lucide-react-native";
import React, { useCallback } from "react";
import { StyleSheet, View } from "react-native";

import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { Skeleton } from "../../../components/ds/Skeleton";
import { TaskRow } from "../../../components/fleet/TaskRow";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";
import type { SnapshotTask } from "../../../lib/ws";

// A first snapshot hasn't arrived yet (fresh connect, reconnecting, dead daemon) — we
// genuinely don't know whether there are tasks. Show the shape of the list instead of
// falsely asserting "no tasks yet" (that claim is only true once the server confirms it).
function TasksSkeleton() {
  return (
    <View style={styles.skeletonWrap}>
      {[0, 1, 2].map((i) => (
        <View key={i} style={styles.skeletonRow}>
          <Skeleton width={18} height={18} radius={9} />
          <Skeleton width="65%" height={15} />
        </View>
      ))}
    </View>
  );
}

export default function SessionTasks() {
  const { snapshot, snapshotTimedOut } = useSessionCtx();
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
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.sessionColumn}>
      {snapshot == null && snapshotTimedOut ? (
        <EmptyState icon={WifiOff} message="can't reach this session — it may not exist, or the server is unreachable" />
      ) : snapshot == null ? (
        <TasksSkeleton />
      ) : (
        <BoundedList
          data={tasks}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          ListEmptyComponent={<EmptyState icon={ListChecks} message="no tasks yet" />}
        />
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  // Hearth desktop/web: the session sub-screens share the chat column's 760px cap,
  // centered in the remaining Fleet+Session pane — a no-op at mobile widths.
  sessionColumn: { width: "100%", maxWidth: 760, alignSelf: "center" },
  skeletonWrap: { paddingTop: space.space12, gap: space.space16 },
  skeletonRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
  },
});
