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
import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef } from "react";
import {
  ActivityIndicator,
  FlatList,
  type FlatListProps,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Platform,
  RefreshControl,
  StyleSheet,
  View,
} from "react-native";

import { haptics } from "../../lib/haptics";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import {
  grewAtOldestEnd,
  offsetFromNewest,
  scrollTopAfterResize,
  scrollTopFor,
  timelineShape,
  type TimelineShape,
} from "../../lib/newestAnchoredScroll";
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
    onScroll: onScrollProp,
    onContentSizeChange: onContentSizeChangeProp,
    ...rest
  }: BoundedListProps<T>,
  ref: React.ForwardedRef<FlatList<T>>,
) {
  const tokens = useTokens();
  const wasRefreshing = useRef(refreshing);
  const listRef = useRef<FlatList<T>>(null);

  useEffect(() => {
    if (wasRefreshing.current && !refreshing) {
      haptics.refreshSettle();
    }
    wasRefreshing.current = refreshing;
  }, [refreshing]);

  // react-native-web renders `inverted` with a scaleY(-1) transform on the scroll container, which
  // flips the scrollbar thumb and fights wheel / trackpad scrolling. On web the list is rendered
  // upright instead (oldest at the top, newest at the bottom) and keeps the inverted contract the
  // callers rely on: scroll events report the offset from the NEWEST end, `scrollToOffset(0)` goes
  // to the newest row, `onEndReached` means "near the oldest row", and an older page landing at
  // the top does not move what the reader is looking at (lib/newestAnchoredScroll.ts).
  const inverted = Boolean((rest as { inverted?: boolean }).inverted);
  const webNatural = Platform.OS === "web" && inverted;
  const listData = useMemo(() => (webNatural && data ? [...data].reverse() : data), [webNatural, data]);
  const fromNewestRef = useRef(0);
  const topGrowthRef = useRef(false);
  const shapeRef = useRef<TimelineShape | null>(null);
  const getNode = useCallback(
    () =>
      (listRef.current as unknown as { getScrollableNode?: () => unknown } | null)?.getScrollableNode?.() as
        | (HTMLElement & { scrollTop: number })
        | null
        | undefined,
    [],
  );
  useLayoutEffect(() => {
    if (!webNatural) return;
    const next = timelineShape((data ?? []).map((item, index) => keyExtractor(item, index)));
    topGrowthRef.current = grewAtOldestEnd(shapeRef.current, next);
    shapeRef.current = next;
  }, [webNatural, data, keyExtractor]);

  const count = data?.length ?? 0;
  useImperativeHandle(
    ref,
    () => {
      const list = listRef.current as FlatList<T>;
      if (!webNatural || !list) return list;
      return {
        // Instant, even when asked to animate: a smooth scroll across thousands of px is
        // interrupted by every older page that lands, and never arrives.
        scrollToOffset: ({ offset }: { offset: number; animated?: boolean }) => {
          const node = getNode();
          if (node) node.scrollTop = scrollTopFor(offset, node.scrollHeight, node.clientHeight);
        },
        scrollToIndex: (params: { index: number; animated?: boolean; viewPosition?: number; viewOffset?: number }) =>
          list.scrollToIndex({
            ...params,
            index: count - 1 - params.index,
            viewPosition: params.viewPosition === undefined ? undefined : 1 - params.viewPosition,
          }),
        scrollToEnd: (params?: { animated?: boolean }) => list.scrollToEnd(params),
        getScrollableNode: () => list.getScrollableNode(),
      } as unknown as FlatList<T>;
    },
    [webNatural, count, getNode],
  );

  const handleScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const ne = e.nativeEvent;
      const from = offsetFromNewest({
        scrollTop: ne.contentOffset.y,
        contentHeight: ne.contentSize.height,
        layoutHeight: ne.layoutMeasurement.height,
      });
      fromNewestRef.current = from;
      if (ne.contentOffset.y < ne.layoutMeasurement.height) onEndReached?.();
      onScrollProp?.({ ...e, nativeEvent: { ...ne, contentOffset: { ...ne.contentOffset, y: from } } });
    },
    [onEndReached, onScrollProp],
  );
  const handleContentSizeChange = useCallback(
    (w: number, h: number) => {
      onContentSizeChangeProp?.(w, h);
      const node = getNode();
      if (!node) return;
      const top = scrollTopAfterResize({
        fromNewest: fromNewestRef.current,
        contentHeight: h,
        layoutHeight: node.clientHeight,
        topGrowth: topGrowthRef.current,
      });
      if (top !== null && Math.abs(node.scrollTop - top) > 0.5) node.scrollTop = top;
    },
    [getNode, onContentSizeChangeProp],
  );

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

  return (
    <View style={styles.fill}>
      <FlatList<T>
        ref={listRef}
        data={(listData as T[]) ?? []}
        renderItem={stableRenderItem}
        keyExtractor={keyExtractor}
        ListEmptyComponent={ListEmptyComponent}
        ListFooterComponent={webNatural ? undefined : footer}
        ListHeaderComponent={webNatural ? footer : undefined}
        onEndReached={webNatural ? undefined : onEndReached}
        onEndReachedThreshold={onEndReachedThreshold}
        contentContainerStyle={[styles.grow, webNatural && styles.newestAtBottom, contentContainerStyle]}
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
        // The default ("never") spends the first tap after typing on dismissing the keyboard: a
        // History search result, say, needed two taps to open. Callers can still override.
        keyboardShouldPersistTaps="handled"
        // Upright on web: every loaded row stays mounted so the newest row is measured and
        // reachable on first paint (a windowed upright list starts at the OLDEST rows).
        maxToRenderPerBatch={webNatural ? Math.max(12, count) : 12}
        windowSize={webNatural ? 1000 : 9}
        initialNumToRender={webNatural ? Math.max(12, count) : 12}
        {...rest}
        inverted={webNatural ? false : inverted}
        onScroll={webNatural ? handleScroll : onScrollProp}
        onContentSizeChange={webNatural ? handleContentSizeChange : onContentSizeChangeProp}
        style={[styles.fill, rest.style, Platform.OS === "web" && (webScrollContain as object)]}
      />
      {Platform.OS === "ios" ? <BellowsSpinner active={refreshing} /> : null}
    </View>
  );
}

export const BoundedList = forwardRef(BoundedListInner) as <T>(
  props: BoundedListProps<T> & { ref?: React.ForwardedRef<FlatList<T>> },
) => React.ReactElement | null;

// Web-only: stops this list's rubber-band from chaining into a page-level bounce
// (same untyped-CSS-passthrough escape hatch as Screen.tsx's ForgeWash `backgroundImage`).
const webScrollContain = { overscrollBehavior: "contain" };

const styles = StyleSheet.create({
  fill: { flex: 1 },
  footer: { paddingVertical: space.space16, alignItems: "center" },
  grow: { flexGrow: 1 },
  newestAtBottom: { justifyContent: "flex-end" },
});
