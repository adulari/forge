// Inbox — sessions waiting on a human (FEATURES.md §4: "waiting is the killer signal";
// the server already sorts waiting sessions first, `useSessions()` just filters them
// down). Hearth: every row IS the elevated decision card (core rule 2) — DecisionCard
// carries its own live-question preview (short-lived socket attach) and Respond/Peek
// actions; DecisionPeek (T4.3) still answers a prompt inline via the peek sheet.
import { CircleCheck } from "lucide-react-native";
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
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

const EVERYTHING_COPY = "That's everything — nothing else needs you.";

function InboxHeader({ count }: { count: number }) {
  const tokens = useTokens();
  return (
    <View style={styles.header}>
      <Text style={[typeScale.title, styles.titleText, { color: tokens.ink }]}>Inbox</Text>
      <Text style={[typeScale.sub, { color: count > 0 ? tokens.danger : tokens.ink3 }]}>
        {count > 0 ? `${count} decision${count === 1 ? "" : "s"} waiting` : EVERYTHING_COPY}
      </Text>
    </View>
  );
}

export default function InboxScreen() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const query = useSessions();
  const rows = useMemo(() => (query.data ?? []).filter((s) => s.waiting), [query.data]);
  const [peekSessionId, setPeekSessionId] = useState<string | null>(null);

  const onPeek = useCallback((row: SessionRow) => setPeekSessionId(row.id), []);
  const closePeek = useCallback(() => setPeekSessionId(null), []);

  const renderItem = useCallback(
    ({ item, index }: { item: SessionRow; index: number }) => <DecisionCard row={item} index={index} onPeek={onPeek} />,
    [onPeek],
  );
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);

  // T5.1 (fixed): expanded's MasterDetail rail (ExpandedRail in (tabs)/_layout.tsx) already
  // renders this same waiting-filtered list via its "Waiting" pill — this screen just fills
  // the detail pane's `<Slot/>` there, so rendering the full Inbox list here too duplicated it.
  if (isExpanded) {
    if (query.isLoading) {
      return <Screen scroll={false}><View style={styles.skeletonRow}><Skeleton width="55%" height={17} /><Skeleton width="70%" height={12} /></View></Screen>;
    }
    if (query.isError) {
      return <Screen scroll={false}><EmptyState icon={CircleCheck} message={query.error instanceof ApiError ? query.error.message : "Could not load waiting sessions."} action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />} /></Screen>;
    }
    return (
      <Screen scroll={false}>
        <EmptyState icon={CircleCheck} message="select a waiting session to see it here." />
      </Screen>
    );
  }

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
        refreshing={query.isRefetching}
        onRefresh={query.refetch}
        contentContainerStyle={styles.listPad}
      />
      <DecisionPeek sessionId={peekSessionId} visible={peekSessionId != null} onClose={closePeek} />
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
