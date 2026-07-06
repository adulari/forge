// T3.4 — Agents segment: `snapshot.subagents` as an AgentCard list/grid (FEATURES.md §1.2
// `subagents` -> Agents segment). 1 column compact/medium, 2 columns expanded (desktop) —
// `key={numColumns}` forces FlatList to remount on that rare breakpoint change, since
// `numColumns` can't change on a live FlatList instance.
import { Bot } from "lucide-react-native";
import React, { useCallback } from "react";
import { StyleSheet, View } from "react-native";

import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { AgentCard } from "../../../components/fleet/AgentCard";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";
import { useBreakpoint } from "../../../theme/useBreakpoint";
import type { SnapshotSubagent } from "../../../lib/ws";

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
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingVertical: space.space12, gap: space.space12 },
  columnWrapper: { gap: space.space12 },
  gridCell: { flex: 1 },
});
