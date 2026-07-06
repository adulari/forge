// T3.4 — Agents segment: `snapshot.subagents` as an AgentCard list/grid (FEATURES.md §1.2
// `subagents` -> Agents segment). 1 column compact/medium, 2 columns expanded (desktop) —
// `key={numColumns}` forces FlatList to remount on that rare breakpoint change, since
// `numColumns` can't change on a live FlatList instance.
import { Bot } from "lucide-react-native";
import React, { useCallback } from "react";
import { StyleSheet, View } from "react-native";

import { BoundedList } from "../../../components/ds/BoundedList";
import { Card } from "../../../components/ds/Card";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { Skeleton } from "../../../components/ds/Skeleton";
import { AgentCard } from "../../../components/fleet/AgentCard";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";
import { useBreakpoint } from "../../../theme/useBreakpoint";
import type { SnapshotSubagent } from "../../../lib/ws";

// Shaped like an AgentCard (icon+name row, task line, meta row) — shown only until the
// first snapshot confirms whether any subagents exist, never claiming "no active agents"
// prematurely (e.g. while still connecting, reconnecting, or on a dead daemon).
function AgentsSkeleton() {
  return (
    <View style={styles.skeletonWrap}>
      {[0, 1].map((i) => (
        <Card key={i} variant="feature">
          <View style={styles.skeletonHeader}>
            <Skeleton width={20} height={20} radius={10} />
            <Skeleton width="40%" height={17} />
          </View>
          <Skeleton width="80%" height={13} style={styles.skeletonGap} />
          <Skeleton width="50%" height={13} style={styles.skeletonGap} />
        </Card>
      ))}
    </View>
  );
}

export default function SessionAgents() {
  const { snapshot } = useSessionCtx();
  const { isExpanded } = useBreakpoint();
  const agents = snapshot?.subagents ?? [];
  const numColumns = isExpanded ? 2 : 1;

  const renderItem = useCallback(
    ({ item }: { item: SnapshotSubagent }) =>
      numColumns > 1 ? (
        <View style={styles.gridCell}>
          <AgentCard agent={item} />
        </View>
      ) : (
        <AgentCard agent={item} />
      ),
    [numColumns],
  );
  // Snapshot.subagents has no stable id — index+agent name is stable enough for this
  // read-only, effectively-append-only list.
  const keyExtractor = useCallback((item: SnapshotSubagent, index: number) => `${index}-${item.agent}`, []);

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false}>
      {snapshot == null ? (
        <AgentsSkeleton />
      ) : (
        <BoundedList
          key={numColumns}
          data={agents}
          numColumns={numColumns}
          columnWrapperStyle={numColumns > 1 ? styles.columnWrapper : undefined}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          ListEmptyComponent={<EmptyState icon={Bot} message="no active agents" />}
          contentContainerStyle={styles.content}
        />
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingVertical: space.space12, gap: space.space12 },
  columnWrapper: { gap: space.space12 },
  gridCell: { flex: 1 },
  skeletonWrap: { paddingVertical: space.space12, paddingHorizontal: space.space16, gap: space.space12 },
  skeletonHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space8 },
});
