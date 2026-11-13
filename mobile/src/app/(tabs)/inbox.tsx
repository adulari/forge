// Inbox — sessions waiting on a human (FEATURES.md §4: "waiting is the killer signal";
// the server already sorts waiting sessions first, `useSessions()` just filters them
// down). Hearth: every row IS the elevated decision card (core rule 2) — DecisionCard
// carries its own live-question preview (short-lived socket attach) and Respond/Peek
// actions; DecisionPeek (T4.3) still answers a prompt inline via the peek sheet.
import { TabPager } from "../../components/TabPager";
import { CircleCheck } from "lucide-react-native";
import { router } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { DecisionCard } from "../../components/cards/DecisionCard";
import { DecisionPeek } from "../../components/cards/DecisionPeek";
import { BoundedList } from "../../components/ds/BoundedList";
import { Button } from "../../components/ds/Button";
import { EmptyState } from "../../components/ds/EmptyState";
import { Screen } from "../../components/ds/Screen";
import { Skeleton } from "../../components/ds/Skeleton";
import { ApiError, type SessionRow } from "../../lib/api";
import { useQueryClient } from "@tanstack/react-query";
import { waitingScoped } from "../../lib/allHostRows";
import { ServerScope } from "../../lib/auth";
import { useSessions } from "../../lib/queries";
import { useAllHostRows } from "../../lib/useAllHostRows";
import type { ScopedSessionRow } from "../../lib/useScopedFleet";
import { activeWorkspaceProjects, useShellPrefs } from "../../lib/shellPrefs";
import { projectName } from "../../lib/sidebarModel";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { usePullRefresh } from "../../lib/usePullRefresh";
import { totalUnseen, useNoticesStore } from "../../lib/notices/store";
import { useBreakpoint } from "../../theme/useBreakpoint";

const EVERYTHING_COPY = "That's everything — nothing else needs you.";

function InboxHeader({ count }: { count: number }) {
  const tokens = useTokens();
  const unseen = totalUnseen(useNoticesStore().hosts);
  // The desktop sidebar/rail already has a Notices button; here it was a full-width bar.
  const { isExpanded } = useBreakpoint();
  return (
    <View style={styles.header}>
      <Text style={[typeScale.title, styles.titleText, { color: tokens.ink }]}>Inbox</Text>
      <Text style={[typeScale.sub, { color: count > 0 ? tokens.danger : tokens.ink3 }]}>
        {count > 0 ? `${count} decision${count === 1 ? "" : "s"} waiting` : "No decisions waiting"}
      </Text>
      {isExpanded ? null : (
        <Button
          label={unseen > 0 ? `Notices (${unseen} new)` : "Notices"}
          variant="secondary"
          onPress={() => router.push("/notices")}
        />
      )}
    </View>
  );
}

export function InboxScreen() {
  const tokens = useTokens();
  const query = useSessions();
  const queryClient = useQueryClient();
  const refreshAll = useCallback(() => queryClient.invalidateQueries({ queryKey: ["sessions"] }), [queryClient]);
  const pull = usePullRefresh(refreshAll);
  const prefs = useShellPrefs();
  const workspaceProjects = activeWorkspaceProjects(prefs);
  const allRows = useAllHostRows();
  const rows = useMemo(
    () => waitingScoped(allRows).filter((s) => !workspaceProjects || workspaceProjects.has(projectName(s.row.cwd))),
    [allRows, workspaceProjects],
  );
  const [peek, setPeek] = useState<{ sessionId: string; serverId: string } | null>(null);

  const onPeek = useCallback(
    (row: SessionRow) => setPeek({ sessionId: row.id, serverId: allRows.find((item) => item.row.id === row.id)?.serverId ?? "" }),
    [allRows],
  );
  const closePeek = useCallback(() => setPeek(null), []);

  const renderItem = useCallback(
    ({ item, index }: { item: ScopedSessionRow; index: number }) => (
      <ServerScope serverId={item.serverId}>
        <DecisionCard row={item.row} index={index} onPeek={onPeek} />
      </ServerScope>
    ),
    [onPeek],
  );
  const keyExtractor = useCallback((item: ScopedSessionRow) => `${item.serverId}:${item.row.id}`, []);

  if (query.isLoading) {
    return (
      <Screen scroll={false} contentContainerStyle={styles.listPad}>
        {[0, 1, 2].map((i) => (
          <View key={i} style={styles.skeletonRow}>
            <Skeleton width="55%" height={17} />
            <Skeleton width="70%" height={12} style={styles.skeletonGap} />
            <Skeleton width="30%" height={12} style={styles.skeletonGap} />
          </View>
        ))}
      </Screen>
    );
  }

  const emptyComponent = query.isError ? (
    <EmptyState
      icon={CircleCheck}
      message={query.error instanceof ApiError ? query.error.message : "something's wrong — couldn't load the inbox."}
      action={<Button label="Retry" variant="secondary" onPress={() => query.refetch()} />}
    />
  ) : (
    <EmptyState icon={CircleCheck} message={EVERYTHING_COPY} />
  );

  return (
    <Screen scroll={false}>
      <InboxHeader count={rows.length} />
      <BoundedList
        data={rows}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
        // The prototype's reassurance line always sits below the decision card(s) once
        // there's at least one — `EmptyState` above already covers the zero-item case.
        ListFooterComponent={!query.isError && rows.length > 0 ? <Text style={[typeScale.sub, styles.footerCopy, { color: tokens.ink4 }]}>{EVERYTHING_COPY}</Text> : undefined}
        refreshing={pull.refreshing}
        onRefresh={pull.onRefresh}
        contentContainerStyle={styles.listPad}
      />
      <ServerScope serverId={peek?.serverId ?? ""}>
        <DecisionPeek sessionId={peek?.sessionId ?? null} visible={peek != null} onClose={closePeek} />
      </ServerScope>
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: { paddingTop: space.space12, gap: space.space4 },
  titleText: { letterSpacing: -0.4 },
  listPad: { paddingTop: space.space16, paddingBottom: space.space32 },
  footerCopy: { paddingHorizontal: space.space16, paddingTop: space.space8 },
  skeletonRow: { paddingHorizontal: space.space16, paddingVertical: space.space16, gap: space.space8 },
  skeletonGap: { marginTop: space.space8 },
});

export default function InboxTab() {
  return (
    <TabPager index={1}>
      <InboxScreen />
    </TabPager>
  );
}
