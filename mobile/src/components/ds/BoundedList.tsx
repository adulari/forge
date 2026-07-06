// DESIGN_SYSTEM.md §6 Containers — BoundedList: FlatList wrapper with stable
// keys, a mandatory ListEmptyComponent, pagination hooks, Bellows pull-to-refresh
// (native), memoized rows.
//
// T6.1: the native RefreshControl still owns the actual pull gesture + refresh
// trigger (reliable cross-platform; there's no safe way to intercept raw
// overscroll distance from a shared FlatList wrapper without real device
// verification). On iOS its default spinner glyph is hidden (`tintColor`
// "transparent" — a supported RefreshControl value; the pull gesture keeps
// working) and BellowsSpinner's ember arc takes over visually the moment
// `refreshing` flips true, rotating for the duration. Android keeps its native
// accent-tinted spinner (RefreshControl there has no fully-transparent tint, and
// stacking a second custom spinner on top would read as a glitch, not polish).
// Settle haptic fires on the refreshing:true -> false edge either way.
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
import { BellowsSpinner } from "./BellowsSpinner";

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
    <View style={styles.fill}>
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
              tintColor={Platform.OS === "ios" ? "transparent" : tokens.accent}
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
      {Platform.OS === "ios" ? <BellowsSpinner active={refreshing} /> : null}
    </View>
  );
}

export const BoundedList = forwardRef(BoundedListInner) as <T>(
  props: BoundedListProps<T> & { ref?: React.ForwardedRef<FlatList<T>> },
) => React.ReactElement | null;

const styles = StyleSheet.create({
  fill: { flex: 1 },
  footer: { paddingVertical: space.space16, alignItems: "center" },
  grow: { flexGrow: 1 },
});
