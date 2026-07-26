// Interactive paging between tabs: the content follows the finger, the neighbouring tab peeks in,
// and a release either settles back or lands on the next tab.
//
// THE PAGER IS A SCROLL VIEW, AND THAT IS THE WHOLE POINT.
//
// The previous attempt drove a translation from a hand-rolled `Gesture.Pan`. It could not stop a
// drag from also being a tap: a horizontal drag across a full-width row stays inside that row's hit
// rect, so RN's `Pressable` retained the press and fired it on release — swiping History → Settings
// opened History's "Resume this session?" dialog. There is no reliable way to reach into RN's
// responder system from a gesture handler and take that press back.
//
// A UIScrollView does it natively. `canCancelContentTouches` (default, set explicitly below because
// it is load-bearing) means that the moment the scroll view decides it is scrolling, it CANCELS the
// touches it has already delivered to its subviews. Dragging across a button cannot press it. The
// vertical axis is settled by the same owner: `directionalLockEnabled` keeps a vertical drag with
// the list inside the page instead of arm-wrestling it, which is what made swipes get stolen.
//
// `pagingEnabled` supplies the peek, the rubber-banding at the first and last tab, and the settle —
// all of it the platform's own physics rather than numbers I picked.
//
// WHAT IS STILL TRUE FROM THE OLD DESIGN
//
// iOS keeps its real tab bar (`RNSTabBarController` is a genuine UITabBarController), and a
// UITabBarController only keeps the SELECTED child's view laid out. So the pager still lives inside
// each tab rather than around the navigator, and a peeked neighbour is still a second instance of
// that screen. Two consequences are handled deliberately:
//
//  · Peeks mount once this tab is settled and are dropped when it loses focus. Mounting them at the
//    START of a drag was too late — a state update plus a lazy import cannot finish inside a quick
//    swipe, so the neighbour slid past empty. Scoping them to the focused tab is what keeps them
//    from outliving their usefulness: no tab holds a live copy of another while you are elsewhere.
//  · They render as peeks (`useIsPeeking`), so they show cached data and ask the network for
//    nothing. A screen sliding past under a thumb is not an arrival.
//
// The page slots exist even when empty, because the scroll view needs their width to be scrollable
// at all — an empty slot is the app's own background, not a light gap.
import { router, useIsFocused } from "expo-router";
import React from "react";
import {
  InteractionManager,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  ScrollView,
  StyleSheet,
  useWindowDimensions,
  View,
} from "react-native";

import { isNative } from "../lib/platform";
import { tabHrefAt, TAB_SWIPE_ORDER } from "../lib/tabSwipe";
import { useTokens } from "../theme/ThemeProvider";
import { TabPeek } from "./TabPeek";

export function TabPager({ index, children }: { index: number; children: React.ReactNode }) {
  const tokens = useTokens();
  const scroller = React.useRef<ScrollView>(null);
  // Mounted BEFORE a drag, not during one. Waiting for `onScrollBeginDrag` meant a state update and
  // a lazy import had to complete inside the swipe itself, and a quick swipe is over first — so the
  // neighbour slid past empty and there was nothing to peek at.
  const [ready, setReady] = React.useState(false);
  // Immediate fallback for a drag that starts before the deferred mount below has run.
  const [peeking, setPeeking] = React.useState(false);
  const isFocused = useIsFocused();
  // Window width rather than a measured layout, because it is known on the FIRST render. Measuring
  // would leave `contentOffset` at zero for that render, which for any tab with a page to its left
  // means the pager opens showing the wrong page and jumps once the measurement lands.
  const { width } = useWindowDimensions();

  const hasPrev = index > 0;
  const hasNext = index < TAB_SWIPE_ORDER.length - 1;
  const homeOffset = hasPrev ? width : 0;

  const unpark = React.useRef<ReturnType<typeof setTimeout>>(undefined);

  const home = React.useCallback(() => {
    scroller.current?.scrollTo({ x: homeOffset, animated: false });
  }, [homeOffset]);

  React.useEffect(() => () => clearTimeout(unpark.current), []);

  // Android ignores the `contentOffset` prop, and a width change (rotation) moves the pages out from
  // under a fixed offset on both platforms.
  React.useEffect(home, [home]);

  // A pager left PARKED on a neighbour page is what showed the previous tab for a frame on arrival:
  // land on Fleet from Inbox, and Inbox's own pager is still sitting on the page that holds Fleet's
  // peek, so returning to Inbox draws Fleet before it snaps back.
  //
  // So it is unparked on arrival, in a LAYOUT effect: that lands the scroll in the same native batch
  // as the tab becoming visible, rather than a frame later where it would be seen. This deliberately
  // does not depend on `useIsFocused` flipping or on `runAfterInteractions` firing — the previous
  // attempt relied on both, and neither can be verified to hold inside NativeTabs from here.
  React.useLayoutEffect(() => {
    if (isFocused) home();
  }, [home, isFocused]);

  // Prepare the neighbours once this tab is settled, so the first drag already has something to
  // reveal — and drop them the moment the tab loses focus, so no tab holds a live copy of another.
  //
  // `runAfterInteractions` rather than a plain effect on purpose: these are `React.lazy` imports, and
  // resolving four route modules inside a first mount is what stopped the app opening once already.
  // Deferring puts that work after the tree exists, where module evaluation expects to happen.
  React.useEffect(() => {
    if (!isFocused) {
      setReady(false);
      return;
    }
    const task = InteractionManager.runAfterInteractions(() => setReady(true));
    return () => task.cancel();
  }, [isFocused]);

  // `peeking` is only the immediate fallback for a drag that beat the deferred mount; clearing it on
  // blur is what stops `showPeeks` staying true and holding neighbours mounted in a tab the user has
  // left. Putting the pager back on its own page is the effect above, deferred, for the flicker
  // reason given there.
  React.useEffect(() => {
    if (!isFocused) setPeeking(false);
  }, [isFocused]);

  const onSettled = React.useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      setPeeking(false);
      if (width === 0) return;
      const landed = Math.round(event.nativeEvent.contentOffset.x / width);
      const homePage = hasPrev ? 1 : 0;
      if (landed === homePage) return;

      const href = tabHrefAt(index + (landed - homePage));
      if (href === null) return;
      router.navigate(href);
      // Unparked on a short delay rather than here. This tab is still on screen for the frames it
      // takes the switch to happen, so snapping back now shows the page just swiped away from
      // immediately before the new tab appears — the flicker in its first form. A switch is a frame
      // or two and no swipe brings you back inside 150ms, so by then this tab is hidden. Belt to the
      // layout effect's braces: a pager that never gets unparked draws the wrong tab on arrival.
      clearTimeout(unpark.current);
      unpark.current = setTimeout(home, 150);
    },
    [hasPrev, home, index, width],
  );

  // A release almost always hands over to the paging animation, and `onMomentumScrollEnd` is the
  // only event that knows where it landed — acting on the drag's own end offset would navigate to a
  // page the animation never settles on. The exception is a release that is already exactly on a
  // boundary, which produces no animation and therefore no momentum event; without this the peek
  // would stay mounted afterwards, and a peek that outlives its drag is what put one tab's confirm
  // dialog on top of another.
  const onEndedDrag = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    if (width === 0) return;
    const past = event.nativeEvent.contentOffset.x % width;
    if (past < 1 || width - past < 1) onSettled(event);
  };

  // Web and the Tauri webview have no touch paging worth the name — a click-drag is a text
  // selection and a trackpad swipe arrives as wheel deltas — so they get the screen unwrapped.
  if (!isNative) return <View style={styles.fill}>{children}</View>;

  const page = { width };
  const showPeeks = ready || peeking;

  return (
    <ScrollView
      ref={scroller}
      horizontal
      pagingEnabled
      // Load-bearing, not decoration: this is what cancels a press when a drag turns into a scroll.
      canCancelContentTouches
      // Keeps a vertical drag with the list inside the page rather than contesting it.
      directionalLockEnabled
      showsHorizontalScrollIndicator={false}
      contentOffset={{ x: homeOffset, y: 0 }}
      onScrollBeginDrag={() => setPeeking(true)}
      onMomentumScrollEnd={onSettled}
      onScrollEndDrag={onEndedDrag}
      style={[styles.fill, { backgroundColor: tokens.bg0 }]}
      contentContainerStyle={styles.content}
    >
      {hasPrev ? <View style={page}>{showPeeks ? <TabPeek at={index - 1} /> : null}</View> : null}
      <View style={page}>{children}</View>
      {hasNext ? <View style={page}>{showPeeks ? <TabPeek at={index + 1} /> : null}</View> : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  // Without this the pages collapse to their content height inside a horizontal scroll view.
  content: { alignItems: "stretch" },
});
