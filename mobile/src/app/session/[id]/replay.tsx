// Replay segment: hosts the shared `ReplayView` (chronological log rows + mono timestamp
// column + scrub bar — see components/session/ReplayView.tsx) against this session's live
// `useHistory` data. Per T3.1 HANDOFF this segment owns its own Screen (edges omit "top" —
// the shell's header/status-strip/Segmented already consumed the top inset) and renders no
// header of its own — the shell's SessionHeader back arrow + "Replay" Segmented tab already
// own that chrome.
import React, { useCallback, useMemo } from "react";
import { StyleSheet } from "react-native";
import { useLocalSearchParams } from "expo-router";

import { ReplayView } from "../../../components/session/ReplayView";
import { Screen } from "../../../components/ds/Screen";
import { useHistory } from "../../../lib/queries";

export default function SessionReplayScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const query = useHistory(id ?? null);
  const rows = useMemo(() => query.data?.pages.flat().slice().reverse() ?? [], [query.data?.pages]);

  const onEndReached = useCallback(() => {
    if (query.hasNextPage && !query.isFetchingNextPage) void query.fetchNextPage();
  }, [query]);

  return (
    <Screen edges={["left", "right", "bottom"]} scroll={false} contentContainerStyle={styles.sessionColumn}>
      <ReplayView
        rows={rows}
        loading={query.isLoading}
        error={query.isError}
        onRetry={() => void query.refetch()}
        refreshing={query.isFetching && !query.isFetchingNextPage}
        onRefresh={() => void query.refetch()}
        onEndReached={onEndReached}
        loadingMore={query.isFetchingNextPage}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  sessionColumn: { width: "100%", maxWidth: 760, alignSelf: "center" },
});
