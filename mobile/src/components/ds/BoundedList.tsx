// DESIGN_SYSTEM.md §6 Containers — BoundedList: FlatList wrapper with stable
// keys, a mandatory ListEmptyComponent, pagination hooks, Bellows pull-to-refresh
// (native), memoized rows.
//
// Bellows note: full pull-distance arc tracking (§5.2) is swept in BUILD_ORDER
// Batch 6 (T6.1 animation & feel pass). This wires a working native refresh loop
// today via RefreshControl (tinted with the accent color) plus the settle haptic
// on the refreshing:true -> false edge — the visual "ember arc" polish layers on
// top of this without changing the props contract.
import React, { forwardRef, useCallback, useEffect, useRef } from "react";
import {
  ActivityIndicator,
  FlatList,
  type FlatListProps,
  Platform,
  RefreshControl,
  StyleSheet,
  View,
} from "react-native";

import { haptics } from "../../lib/haptics";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";

export interface BoundedListProps<T>
  extends Omit<FlatListProps<T>, "renderItem" | "keyExtractor" | "ListEmptyComponent" | "data"> {
  data: readonly T[] | null | undefined;
  renderItem: (info: { item: T; index: number }) => React.ReactElement | null;
  keyExtractor: (item: T, index: number) => string;
  /** Required: every list needs a designed empty state (DESIGN_SYSTEM §4). */
  ListEmptyComponent: React.ReactElement;
  /** Pagination: called near the end of the list. */
  onEndReached?: () => void;
  onEndReachedThreshold?: number;
  /** Shows a footer spinner while fetching the next page. */
  loadingMore?: boolean;
  refreshing?: boolean;
  onRefresh?: () => void;
}

function BoundedListInner<T>(
  {
    data,
    renderItem,
    keyExtractor,
    ListEmptyComponent,
    onEndReached,
    onEndReachedThreshold = 0.4,
    loadingMore = false,
    refreshing = false,
    onRefresh,
    contentContainerStyle,
    ...rest
  }: BoundedListProps<T>,
  ref: React.ForwardedRef<FlatList<T>>,
) {
  const tokens = useTokens();
  const wasRefreshing = useRef(refreshing);

  useEffect(() => {
    if (wasRefreshing.current && !refreshing) {
      haptics.refreshSettle();
    }
    wasRefreshing.current = refreshing;
  }, [refreshing]);

  // Stable identity + row purity (row components callers pass in should be
  // React.memo'd) together satisfy "memoized rows" without fighting FlatList's
  // own virtualization.
  const stableRenderItem = useCallback(
    ({ item, index }: { item: T; index: number }) => renderItem({ item, index }),
    [renderItem],
  );

  const footer = loadingMore ? (
    <View style={styles.footer}>
      <ActivityIndicator color={tokens.accent} />
    </View>
  ) : undefined;

  const isEmpty = !data || data.length === 0;

  return (
    <FlatList<T>
      ref={ref}
      data={(data as T[]) ?? []}
      renderItem={stableRenderItem}
      keyExtractor={keyExtractor}
      ListEmptyComponent={ListEmptyComponent}
      ListFooterComponent={footer}
      onEndReached={onEndReached}
      onEndReachedThreshold={onEndReachedThreshold}
      contentContainerStyle={[isEmpty && styles.grow, contentContainerStyle]}
      refreshControl={
        Platform.OS !== "web" && onRefresh ? (
          <RefreshControl
            refreshing={refreshing}
            onRefresh={onRefresh}
            tintColor={tokens.accent}
            colors={[tokens.accent]}
          />
        ) : undefined
      }
      removeClippedSubviews={Platform.OS !== "web"}
      maxToRenderPerBatch={12}
      windowSize={9}
      initialNumToRender={12}
      {...rest}
    />
  );
}

export const BoundedList = forwardRef(BoundedListInner) as <T>(
  props: BoundedListProps<T> & { ref?: React.ForwardedRef<FlatList<T>> },
) => React.ReactElement | null;

const styles = StyleSheet.create({
  footer: { paddingVertical: space.space16, alignItems: "center" },
  grow: { flexGrow: 1 },
});
