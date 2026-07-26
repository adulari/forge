// Interactive paging between tabs — the content follows the finger, the neighbouring tab peeks in
// behind it, and a released drag either springs back or completes. Replaces the commit-only gesture
// in TabSwipe.tsx, which switched tabs at a threshold with nothing to see on the way.
//
// WHY THE PEEK LIVES INSIDE ONE TAB
//
// iOS keeps its real tab bar here (`RNSTabBarController` is a genuine UITabBarController — verified
// in react-native-screens/ios/tabs/host), and a UITabBarController only keeps the SELECTED child's
// view laid out. There is no supported way to page between its children: `NativeTabs` has no swipe
// surface at all, and react-native-screens' `gesture-handler/` support is for stack transitions.
//
// So the pager is rendered by every tab route rather than around the navigator: the selected tab
// draws itself in the middle of a three-page row and its neighbours to either side, and the drag
// translates that row. The native bar never moves, is never reimplemented, and keeps its Liquid
// Glass material, scroll-to-minimize and native badges.
//
// TWO HONEST ARTIFACTS of doing it this way, neither of which the bar can be traded for:
//
//  · The neighbour drawn during a drag is a SECOND instance of that screen, so it starts at the top
//    of its list. Committing hands over to the tab's own instance, which restores its real scroll
//    offset — visible as a jump only if you had scrolled that tab and then swiped to it.
//  · The bar's selected item updates when the swipe commits, not continuously with the finger. A
//    native UITabBar exposes no partial-selection state to animate.
//
// Neighbours mount lazily and only while a drag is in flight, so the idle cost is zero pages and
// the peak is two extra screens.
import { router } from "expo-router";
import React from "react";
import { StyleSheet, useWindowDimensions, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  useReducedMotion,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import { haptics } from "../lib/haptics";
import { isNative } from "../lib/platform";
import { tabHrefAt, TAB_SWIPE_ORDER } from "../lib/tabSwipe";
import { TabPeek } from "./TabPeek";

/** Horizontal distance before the pager takes the drag — clear of SessionCard's 10px archive pan. */
const ACTIVATE_X = 28;
/** Vertical slop that hands the drag back to a scrolling list. */
const FAIL_Y = 15;
/** Fraction of the screen a drag must cross to complete on distance alone. */
const COMMIT_FRACTION = 0.32;
/** Or this much horizontal speed, so a short flick still completes. */
const COMMIT_VELOCITY = 520;
/** How much of a drag past the first/last tab is shown, as resistance rather than a dead stop. */
const OVERSCROLL = 0.2;

export function TabPager({ index, children }: { index: number; children: React.ReactNode }) {
  const { width } = useWindowDimensions();
  const reduced = useReducedMotion();
  const drag = useSharedValue(0);
  // Neighbours are only worth mounting while a drag can actually reveal them.
  const [peeking, setPeeking] = React.useState(false);

  const hasPrev = index > 0;
  const hasNext = index < TAB_SWIPE_ORDER.length - 1;

  const go = React.useCallback((target: number) => {
    const href = tabHrefAt(target);
    if (href === null) return;
    haptics.select();
    // The next route mounts its own pager at rest in the centre, so the row must be back at zero
    // before the swap or the incoming tab would start life translated by a screen width.
    drag.value = 0;
    setPeeking(false);
    router.navigate(href);
  }, [drag]);

  const settle = React.useCallback(() => setPeeking(false), []);

  const pan = React.useMemo(
    () =>
      Gesture.Pan()
        .enabled(isNative)
        .activeOffsetX([-ACTIVATE_X, ACTIVATE_X])
        .failOffsetY([-FAIL_Y, FAIL_Y])
        .onBegin(() => {
          runOnJS(setPeeking)(true);
        })
        .onUpdate((event) => {
          const x = event.translationX;
          // Dragging toward an edge that has no tab still moves, just reluctantly — a hard stop
          // reads as a broken gesture, and the resistance is what says "nothing that way".
          const blocked = (x > 0 && !hasPrev) || (x < 0 && !hasNext);
          drag.value = blocked ? x * OVERSCROLL : x;
        })
        .onEnd((event) => {
          const far = Math.abs(event.translationX) > width * COMMIT_FRACTION;
          const fast = Math.abs(event.velocityX) > COMMIT_VELOCITY;
          const toPrev = event.translationX > 0;
          const allowed = toPrev ? hasPrev : hasNext;

          if (!(far || fast) || !allowed) {
            drag.value = reduced ? 0 : withSpring(0, springBack);
            runOnJS(settle)();
            return;
          }

          const target = toPrev ? index - 1 : index + 1;
          const destination = toPrev ? width : -width;
          if (reduced) {
            runOnJS(go)(target);
            return;
          }
          // Timing, not a spring: the row has to ARRIVE at exactly one page so the handover to the
          // real screen happens with nothing mid-flight. A spring's overshoot would show a sliver
          // of the page beyond the target.
          drag.value = withTiming(destination, completeTiming, (finished) => {
            if (finished) runOnJS(go)(target);
          });
        }),
    [drag, go, hasNext, hasPrev, index, reduced, settle, width],
  );

  const rowStyle = useAnimatedStyle(() => ({ transform: [{ translateX: drag.value }] }));

  return (
    <GestureDetector gesture={pan}>
      <View style={styles.clip}>
        <Animated.View style={[styles.layer, rowStyle]}>
          {peeking && hasPrev ? (
            <View style={[styles.page, { width, left: -width }]}>
              <TabPeek at={index - 1} />
            </View>
          ) : null}
          <View style={[styles.page, { width, left: 0 }]}>{children}</View>
          {peeking && hasNext ? (
            <View style={[styles.page, { width, left: width }]}>
              <TabPeek at={index + 1} />
            </View>
          ) : null}
        </Animated.View>
      </View>
    </GestureDetector>
  );
}

const springBack = { damping: 26, stiffness: 320 } as const;
const completeTiming = { duration: 220 } as const;

const styles = StyleSheet.create({
  // The clip is what makes this a pager rather than three screens side by side: neighbours sit
  // outside these bounds until the drag brings them in.
  clip: { flex: 1, overflow: "hidden" },
  layer: { flex: 1 },
  page: { position: "absolute", top: 0, bottom: 0 },
});
