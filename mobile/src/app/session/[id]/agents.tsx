// T3.4 — Agents segment: `snapshot.subagents` as a de-boxed hairline list (FEATURES.md §1.2
// `subagents` -> Agents segment; Hearth core rule 1 — a running agent carries the accent
// HeatEdge instead of living in its own Card, see AgentRow.tsx).
import { Bot, WifiOff } from "lucide-react-native";
import React, { useCallback } from "react";
import { StyleSheet, View } from "react-native";

import { AgentRow } from "../../../components/session/AgentRow";
import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { Skeleton } from "../../../components/ds/Skeleton";
import { useSessionCtx } from "../../../lib/sessionContext";
import { space } from "../../../theme/tokens";
import type { SnapshotSubagent } from "../../../lib/ws";

// Shaped like an AgentRow (dot+name+cost header, task line, meta line) — shown only until
// the first snapshot confirms whether any subagents exist, never claiming "no active agents"
// prematurely (e.g. while still connecting, reconnecting, or on a dead daemon).
function AgentsSkeleton() {
  return (
    <View style={styles.skeletonWrap}>
      {[0, 1].map((i) => (
        <View key={i} style={styles.skeletonRow}>
          <View style={styles.skeletonHeader}>
            <Skeleton width={8} height={8} radius={4} />
            <Skeleton width="40%" height={17} />
          </View>
          <Skeleton width="80%" height={13} style={styles.skeletonGap} />
          <Skeleton width="50%" height={13} style={styles.skeletonGap} />
        </View>
      ))}
    </View>
  );
}

export default function SessionAgents() {
  const { snapshot, snapshotTimedOut } = useSessionCtx();
  const agents = snapshot?.subagents ?? [];

  const renderItem = useCallback(
    ({ item }: { item: SnapshotSubagent }) => <AgentRow agent={item} />,
    [],
  );
  // Snapshot.subagents has no stable id — index+agent name is stable enough for this
  // read-only, effectively-append-only list.
  const keyExtractor = useCallback((item: SnapshotSubagent, index: number) => `${index}-${item.agent}`, []);

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.sessionColumn}>
      {snapshot == null && snapshotTimedOut ? (
        <EmptyState icon={WifiOff} message="can't reach this session — it may not exist, or the server is unreachable" />
      ) : snapshot == null ? (
        <AgentsSkeleton />
      ) : (
        <BoundedList
          data={agents}
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
  sessionColumn: { width: "100%", maxWidth: 760, alignSelf: "center" },
  content: { paddingTop: space.space8 },
  skeletonWrap: { paddingVertical: space.space12, gap: space.space20 },
  skeletonRow: { paddingHorizontal: space.space20, gap: space.space8 },
  skeletonHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
});
