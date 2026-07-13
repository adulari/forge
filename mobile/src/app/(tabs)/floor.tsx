import { Flame } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { FloorTile } from "../../components/floor/FloorTile";
import { Button } from "../../components/ds/Button";
import { EmptyState } from "../../components/ds/EmptyState";
import { Screen } from "../../components/ds/Screen";
import { BoundedList } from "../../components/ds/BoundedList";
import type { SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

const SOCKET_CAP = 8;

export default function FloorScreen() {
  const tokens = useTokens();
  const { width } = useBreakpoint();
  const query = useSessions();
  const [visibleIds, setVisibleIds] = useState<Set<string>>(new Set());
  const burning = useMemo(() => (query.data ?? []).filter((row) => row.waiting || row.busy), [query.data]);
  const columns = width >= 1024 ? 3 : width >= 768 ? 2 : 1;
  const renderItem = useCallback(({ item }: { item: SessionRow }) => <View style={styles.tileWrap}><FloorTile row={item} active={visibleIds.has(item.id)} /></View>, [visibleIds]);
  const keyExtractor = useCallback((item: SessionRow) => item.id, []);
  const onViewableItemsChanged = useCallback(({ viewableItems }: { viewableItems: { item: SessionRow }[] }) => setVisibleIds(new Set(viewableItems.slice(0, SOCKET_CAP).map(({ item }) => item.id))), []);

  if (query.isLoading) return <Screen><View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Loading live sessions…</Text></View></Screen>;

  return <Screen scroll={false}>
    <View style={styles.header}><Flame size={20} strokeWidth={1.75} color={tokens.accent} /><Text style={[typeScale.title, { color: tokens.ink }]}>The Floor</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{burning.length} burning</Text></View>
    <BoundedList key={`floor-${columns}`} data={burning} renderItem={renderItem} keyExtractor={keyExtractor} numColumns={columns} onViewableItemsChanged={onViewableItemsChanged} refreshing={query.isRefetching} onRefresh={() => void query.refetch()} ListEmptyComponent={query.isError ? <EmptyState icon={Flame} message="Could not load live sessions." action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} />} /> : <EmptyState icon={Flame} message="The floor is cool — no live sessions right now." />} contentContainerStyle={styles.list} />
  </Screen>;
}

const styles = StyleSheet.create({ header: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 }, list: { paddingBottom: space.space24 }, tileWrap: { flex: 1, padding: space.space4 }, loading: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space12 } });
