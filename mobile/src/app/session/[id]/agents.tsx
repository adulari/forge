// T3.4 — Agents segment: `snapshot.subagents` as a de-boxed hairline list (FEATURES.md §1.2
// `subagents` -> Agents segment; Hearth core rule 1 — a running agent carries its accent
// StatusDot instead of living in its own Card, see AgentRow.tsx).
import { Bot, WifiOff } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { openSession } from "../../../lib/nav";
import { AgentRow } from "../../../components/session/AgentRow";
import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { Skeleton } from "../../../components/ds/Skeleton";
import { useSessionAgents } from "../../../lib/queries";
import { useSessionCtx } from "../../../lib/sessionContext";
import { agentListItems, type AgentListItem } from "../../../lib/agentList";
import { useTokens } from "../../../theme/ThemeProvider";
import { type as typeScale } from "../../../theme/typography";
import { space } from "../../../theme/tokens";
import { useSegmentsHosted } from "../../../components/session/SegmentKeepAlive";

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

export function SessionAgents() {
  const tokens = useTokens();
  const { sessionId, snapshot, snapshotTimedOut, send } = useSessionCtx();
  const live = snapshot?.subagents;
  const promptSeq = snapshot?.prompt_seq;
  const signal = (live ?? []).map((s) => (s.done ? "d" : "r")).join("");
  const index = useSessionAgents(sessionId, signal).data;
  const items = useMemo(() => agentListItems(live ?? [], index ?? []), [live, index]);

  // A child agent never parks on a permission prompt today (a child resolves `Ask` as deny), so
  // `permission_prompt` is always null and these props stay inert — they are passed so the inline
  // Allow/Deny answers through the same `allow` message the session-level card uses if that
  // changes, rather than needing a call-site edit then.
  const renderItem = useCallback(
    ({ item }: { item: AgentListItem }) =>
      item.kind === "header" ? (
        <Text style={[typeScale.section, styles.header, { color: tokens.ink4 }]}>{item.label}</Text>
      ) : (
        <AgentRow
          agent={item.agent}
          send={send}
          promptSeq={promptSeq}
          onOpen={item.agent.session_id ? () => openSession(item.agent.session_id as string) : undefined}
        />
      ),
    [send, promptSeq, tokens.ink4],
  );
  const keyExtractor = useCallback((item: AgentListItem) => item.key, []);

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.sessionColumn}>
      {snapshot == null && snapshotTimedOut ? (
        <EmptyState icon={WifiOff} message="can't reach this session — it may not exist, or the server is unreachable" />
      ) : snapshot == null ? (
        <AgentsSkeleton />
      ) : (
        <BoundedList
          data={items}
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
  header: { paddingHorizontal: space.space4, paddingTop: space.space12, paddingBottom: space.space8 },
  skeletonWrap: { paddingVertical: space.space12, gap: space.space20 },
  skeletonRow: { paddingHorizontal: space.space20, gap: space.space8 },
  skeletonHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  skeletonGap: { marginTop: space.space4 },
});

// Desktop/web: the session layout keeps this segment mounted itself (SegmentKeepAlive), so the
// route renders nothing there; phones render the screen as the route.
export default function SessionAgentsRoute() {
  return useSegmentsHosted() ? null : <SessionAgents />;
}
