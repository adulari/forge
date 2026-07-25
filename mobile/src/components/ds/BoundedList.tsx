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
import React, { forwardRef, useCallback, useEffect, useImperativeHandle, useRef } from "react";
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

// Approximate CSS pixel height of one "line" for DOM_DELTA_LINE wheel normalization below —
// matches the common browser default (16px root font-size × ~1 line-height).
const WHEEL_LINE_HEIGHT_PX = 16;

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
  const listRef = useRef<FlatList<T>>(null);
  useImperativeHandle(ref, () => listRef.current as FlatList<T>);

  useEffect(() => {
    if (wasRefreshing.current && !refreshing) {
      haptics.refreshSettle();
    }
    wasRefreshing.current = refreshing;
  }, [refreshing]);

  // react-native-web renders `inverted` with a scaleY(-1) transform but does NOT flip wheel
  // events to match, so mouse/trackpad scrolling fights the list (the classic glitchy
  // inverted-chat scroll). Take over the wheel entirely on web while inverted: flip deltaY
  // into the scroll position ourselves.
  const inverted = Boolean((rest as { inverted?: boolean }).inverted);
  useEffect(() => {
    if (Platform.OS !== "web" || !inverted) return;
    const node = (listRef.current as unknown as { getScrollableNode?: () => unknown })?.getScrollableNode?.() as
      | (HTMLElement & { scrollTop: number })
      | null
      | undefined;
    if (!node || typeof node.addEventListener !== "function") return;
    const onWheel = (e: WheelEvent) => {
      // Horizontal intent (shift+wheel, trackpad pan) must reach the browser's native
      // handling instead of being swallowed here — otherwise a nested horizontal scroller
      // (e.g. CodeBlock's code-line ScrollView) can never be shift-wheel scrolled, since this
      // listener sits on the ancestor the event bubbles through on its way up.
      if (e.deltaX !== 0) return;
      e.preventDefault();
      // `deltaY` is only pixels on Chrome/Safari/WebView2 (DOM_DELTA_PIXEL). Firefox reports a
      // classic mouse wheel as DOM_DELTA_LINE (~3 "lines" per notch) and a few environments use
      // DOM_DELTA_PAGE — without normalizing, Firefox scrolls the transcript ~3px per notch.
      let deltaY = e.deltaY;
      if (e.deltaMode === WheelEvent.DOM_DELTA_LINE) deltaY *= WHEEL_LINE_HEIGHT_PX;
      else if (e.deltaMode === WheelEvent.DOM_DELTA_PAGE) deltaY *= node.clientHeight;
      node.scrollTop -= deltaY;
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => node.removeEventListener("wheel", onWheel);
  }, [inverted]);

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
        data={(data as T[]) ?? []}
        renderItem={stableRenderItem}
        keyExtractor={keyExtractor}
        ListEmptyComponent={ListEmptyComponent}
        ListFooterComponent={footer}
        onEndReached={onEndReached}
        onEndReachedThreshold={onEndReachedThreshold}
        contentContainerStyle={[styles.grow, contentContainerStyle]}
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
});
