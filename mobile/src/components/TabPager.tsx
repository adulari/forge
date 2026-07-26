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
// Neighbours mount on the first drag and stay, so only that first swipe waits on a render.
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

import { isNative } from "../lib/platform";
import {
  ACTIVATE_X,
  COMMIT_FRACTION,
  COMMIT_VELOCITY,
  FAIL_Y,
  OVERSCROLL,
} from "../lib/tabGesture";
import { tabHrefAt, TAB_SWIPE_ORDER } from "../lib/tabSwipe";
import { useTokens } from "../theme/ThemeProvider";
import { TabPeek } from "./TabPeek";

// A thumb swipe arcs. The thresholds first shipped demanded |dx| > 28 before |dy| > 15 — a ratio of
// nearly 2:1 horizontal-to-vertical just to start — so an ordinary curved swipe crossed the vertical
// limit first, the gesture failed, and the list underneath scrolled instead. That is the "sometimes
// it scrolls the page" case.
//
// The numbers and the relationships between them live in lib/tabGesture.ts, where they are asserted:
// the pager must cancel later vertically than it activates horizontally, and must stay clear of
// SessionCard's own pan. Worklets below close over them, which is safe — constants capture by value.

export function TabPager({ index, children }: { index: number; children: React.ReactNode }) {
  const { width } = useWindowDimensions();
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const drag = useSharedValue(0);
  // Sticky: mounted by the first drag and kept. Unmounting on every settle meant each swipe waited
  // on `runOnJS` -> setState -> render before the neighbour existed, and the first frames of the drag
  // exposed an empty slot. Keeping them costs two idle screens that ask the network for nothing (see
  // `useIsPeeking`), which is what a pager would hold anyway.
  const [peeked, setPeeked] = React.useState(false);

  const hasPrev = index > 0;
  const hasNext = index < TAB_SWIPE_ORDER.length - 1;

  const go = React.useCallback((target: number) => {
    const href = tabHrefAt(target);
    if (href === null) return;
    // No haptic here on purpose. Tapping a native tab bar does not buzz, so a swipe that reaches the
    // same place should not either — landing on the tab you dragged to is not news.
    //
    // The next route mounts its own pager at rest in the centre, so the row must be back at zero
    // before the swap or the incoming tab would start life translated by a screen width.
    drag.value = 0;
    router.navigate(href);
  }, [drag]);

  const pan = React.useMemo(
    () =>
      Gesture.Pan()
        .enabled(isNative)
        .activeOffsetX([-ACTIVATE_X, ACTIVATE_X])
        .failOffsetY([-FAIL_Y, FAIL_Y])
        .onBegin(() => {
          runOnJS(setPeeked)(true);
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
    [drag, go, hasNext, hasPrev, index, reduced, width],
  );

  const rowStyle = useAnimatedStyle(() => ({ transform: [{ translateX: drag.value }] }));

  return (
    <GestureDetector gesture={pan}>
      {/* Opaque, and that is the fix for the flicker rather than a nicety. Anything this container
          does not paint falls through to the native view behind it, which is a light #F0F0F0 — a
          screen recording caught it as a full-frame flash for one frame at the handover and as light
          strips down the incoming edge mid-drag. Painting the app's own background here means an
          unfilled moment reads as the app, not as a light gap. */}
      <View style={[styles.clip, { backgroundColor: tokens.bg0 }]}>
        <Animated.View style={[styles.layer, rowStyle]}>
          {peeked && hasPrev ? (
            <View style={[styles.page, { width, left: -width }]}>
              <TabPeek at={index - 1} />
            </View>
          ) : null}
          <View style={[styles.page, { width, left: 0 }]}>{children}</View>
          {peeked && hasNext ? (
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
