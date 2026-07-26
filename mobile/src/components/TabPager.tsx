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
//  · Peeks mount once this tab is settled and then STAY. Mounting them at the start of a drag was
//    too late — a state update plus a lazy import cannot finish inside a quick swipe, so the
//    neighbour slid past empty. Dropping them on blur was worse: every arrival then re-mounted the
//    previous tab's screen into the page beside this one, which is real work landing on the exact
//    frame the tab becomes visible.
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
import { landedTabHref, pagerGeometry, TAB_SWIPE_ORDER } from "../lib/tabSwipe";
import { useTokens } from "../theme/ThemeProvider";
import { TabPeek } from "./TabPeek";

export function TabPager({ index, children }: { index: number; children: React.ReactNode }) {
  const tokens = useTokens();
  const scroller = React.useRef<ScrollView>(null);
  // Mounted BEFORE a drag, not during one. Waiting for `onScrollBeginDrag` meant a state update and
  // a lazy import had to complete inside the swipe itself, and a quick swipe is over first — so the
  // neighbour slid past empty and there was nothing to peek at.
  const [ready, setReady] = React.useState(false);
  // Immediate fallback for a drag that starts before the deferred mount below has run. Also a
  // one-way latch: once neighbours exist there is nothing to turn off, and turning them off is what
  // put a screen mount on the arrival frame.
  const [peeking, setPeeking] = React.useState(false);
  const isFocused = useIsFocused();
  // Window width rather than a measured layout, because it is known on the FIRST render. Measuring
  // would leave `contentOffset` at zero for that render, which for any tab with a page to its left
  // means the pager opens showing the wrong page and jumps once the measurement lands.
  const { width } = useWindowDimensions();

  const hasPrev = index > 0;
  const hasNext = index < TAB_SWIPE_ORDER.length - 1;
  const { homePage, homeOffset, contentWidth } = pagerGeometry(index, width);

  // Set for the duration of a drag and its settle, so the recovery in `onLayout` cannot yank the
  // pages out from under a finger if something else changes this view's size mid-swipe.
  const dragging = React.useRef(false);

  const home = React.useCallback(() => {
    scroller.current?.scrollTo({ x: homeOffset, animated: false });
  }, [homeOffset]);

  // Homed on every FOCUS CHANGE, rather than on a delay after navigating.
  //
  // The delay was the fourth attempt at the same flash and, like the three before it, it treated a
  // symptom: it assumed the pager was left parked on a neighbour page and only needed putting back
  // before its tab was next seen. Shortening it made the flash shorter, which read like progress and
  // was really just evidence that the corrective scroll was racing something. The clamp described in
  // `pagerGeometry` is that something, and it happens inside layout — earlier than any scroll this
  // component can schedule, which is why no delay was ever going to be the answer.
  //
  // Homing on focus change is what remains once the clamp is gone: cheap, with nothing to tune, and
  // it covers both ends. On blur this view is still attached, and blur comes from the same navigation
  // state change that moves the tab controller to its new child, so the snap lands in that commit
  // rather than in a frame where this tab is still the visible one — the flaw in snapping back inside
  // `onSettled`, which runs before the navigation has taken effect at all. On focus it is the
  // backstop for a pager that arrives somewhere unexpected regardless.
  //
  // Also covers mount (Android ignores the `contentOffset` prop) and a width change from rotation
  // moving the pages out from under a fixed offset.
  React.useLayoutEffect(() => {
    home();
  }, [home, isFocused]);

  // Prepare the neighbours once this tab is settled, and then LEAVE THEM MOUNTED. A one-way latch,
  // not a focus toggle.
  //
  // Unmounting them on blur meant every arrival re-mounted the previous tab's screen into the page
  // beside this one, and mounting a whole screen there is real work happening at the exact moment
  // the tab becomes visible — which is when the flash was seen. Nothing mounts on arrival now.
  //
  // Keeping them was previously unsafe because a drag could tap through to a row and open a dialog
  // that then outlived its tab. It cannot now: the scroll view cancels touches in its subviews the
  // moment it scrolls, and an off-screen page receives none in the first place.
  //
  // `runAfterInteractions` rather than a plain effect on purpose: these are `React.lazy` imports, and
  // resolving four route modules inside a first mount is what stopped the app opening once already.
  // Deferring puts that work after the tree exists, where module evaluation expects to happen.
  React.useEffect(() => {
    if (!isFocused || ready) return;
    const task = InteractionManager.runAfterInteractions(() => setReady(true));
    return () => task.cancel();
  }, [isFocused, ready]);

  const onSettled = React.useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      dragging.current = false;
      if (width === 0) return;
      const landed = Math.round(event.nativeEvent.contentOffset.x / width);
      // `null` means it settled back where it started: nothing to navigate to, nothing to unpark.
      const href = landedTabHref(index, landed, homePage);
      if (href === null) return;
      // Deliberately does NOT unpark here. This tab is still the visible one for the frames the
      // switch takes, so snapping back now shows the page just swiped away from immediately before
      // the new tab appears. The blur that this navigation causes does it instead.
      router.navigate(href);
    },
    [homePage, index, width],
  );

  // Third line of defence, and the only one tied to the layout pass the clamp lives in. Layout is
  // where a scroll view reconciles its offset against its content size, so if anything ever measures
  // this content short again, `onLayout` is the first callback after it — ahead of display on iOS,
  // where layout precedes drawing. Cheap and idempotent: in the ordinary case the pager is already
  // home and the scroll is a no-op.
  const onLayout = React.useCallback(() => {
    if (!dragging.current) home();
  }, [home]);

  // A release almost always hands over to the paging animation, and `onMomentumScrollEnd` is the
  // only event that knows where it landed — acting on the drag's own end offset would navigate to a
  // page the animation never settles on. The exception is a release that is already exactly on a
  // boundary, which produces no animation and therefore no momentum event; without this the peek
  // would stay mounted afterwards, and a peek that outlives its drag is what put one tab's confirm
  // dialog on top of another.
  const onEndedDrag = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    // Cleared here as well as in `onSettled`, because a release that produces no paging animation
    // produces no momentum event either — and a drag flag left set would disable the recovery above
    // for the rest of this tab's life.
    dragging.current = false;
    if (width === 0) return;
    const past = event.nativeEvent.contentOffset.x % width;
    if (past < 1 || width - past < 1) onSettled(event);
  };

  const onBeganDrag = () => {
    dragging.current = true;
    setPeeking(true);
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
      onLayout={onLayout}
      onScrollBeginDrag={onBeganDrag}
      onMomentumScrollEnd={onSettled}
      onScrollEndDrag={onEndedDrag}
      style={[styles.fill, { backgroundColor: tokens.bg0 }]}
      // Pinned rather than measured — see `pagerGeometry`. This is the flash.
      contentContainerStyle={[styles.content, { width: contentWidth }]}
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
