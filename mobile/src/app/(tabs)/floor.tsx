import { Flame } from "lucide-react-native";
import React, { useCallback, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { FloorTile } from "../../components/floor/FloorTile";
import { EmptyState } from "../../components/ds/EmptyState";
import { Screen } from "../../components/ds/Screen";
import { SessionCard } from "../../components/fleet/SessionCard";
import { BoundedList } from "../../components/ds/BoundedList";
import type { SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

const SOCKET_CAP = 8;
type FloorItem = { kind: "tile"; row: SessionRow; active: boolean } | { kind: "row"; row: SessionRow; index: number };

export default function FloorScreen() {
  const tokens = useTokens();
  const { width } = useBreakpoint();
  const { data } = useSessions();
  const [visibleIds, setVisibleIds] = useState<Set<string>>(new Set());
  const sessions = data ?? [];
  const burning = sessions.filter((row) => row.waiting || row.busy);
  const cooled = sessions.filter((row) => !row.waiting && !row.busy);
  const items = useMemo<FloorItem[]>(() => [
    ...burning.map((row) => ({ kind: "tile" as const, row, active: visibleIds.has(row.id) })),
    ...cooled.map((row, index) => ({ kind: "row" as const, row, index })),
  ], [burning, cooled, visibleIds]);
  const columns = width >= 1024 ? 3 : width >= 768 ? 2 : 1;
  const renderItem = useCallback(({ item }: { item: FloorItem }) => item.kind === "tile" ? <View style={[styles.tileWrap, { width: `${100 / columns}%` }]}><FloorTile row={item.row} active={item.active} /></View> : <SessionCard row={item.row} index={item.index} />, [columns]);
  const keyExtractor = useCallback((item: FloorItem) => item.row.id, []);
  const onViewableItemsChanged = useCallback(({ viewableItems }: { viewableItems: Array<{ item: FloorItem }> }) => {
    setVisibleIds(new Set(viewableItems.filter(({ item }) => item.kind === "tile").slice(0, SOCKET_CAP).map(({ item }) => item.row.id)));
  }, []);

  return <Screen scroll={false}>
    <View style={styles.header}><Flame size={20} strokeWidth={1.75} color={tokens.accent} /><Text style={[typeScale.title, { color: tokens.ink }]}>The Floor</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{burning.length} burning</Text></View>
    <BoundedList data={items} renderItem={renderItem} keyExtractor={keyExtractor} numColumns={columns} onViewableItemsChanged={onViewableItemsChanged} ListEmptyComponent={<EmptyState icon={Flame} message="the floor is cool — no live sessions right now." />} contentContainerStyle={styles.list} />
  </Screen>;
}

const styles = StyleSheet.create({ header: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 }, list: { paddingBottom: space.space24 }, tileWrap: { padding: space.space4 } });
