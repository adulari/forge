// Horizontal swipe anywhere on a tab screen switches to the neighbouring tab. The switch is
// immediate: no peek, no content following the finger.
//
// IT USED TO PEEK, AND THAT IS WHY IT NO LONGER DOES.
//
// iOS keeps its real tab bar here — `RNSTabBarController` is a genuine UITabBarController — and a
// UITabBarController only keeps the SELECTED child's view laid out. So peeking meant rendering a
// SECOND live instance of the neighbouring screen inside the current tab, which leaked in every
// direction:
//
//  · Accidental taps. A horizontal drag across a full-width row stays inside that row's hit rect, so
//    RN's `Pressable` retains the press and fires it on release. Swiping History → Settings opened
//    History's "Resume this session?" confirm dialog.
//  · Worse, that dialog then floated OVER the Settings tab, because a peek that stays mounted keeps
//    its state — including open modals — alive in a tab it does not belong to.
//  · Duplicate fetches and loading states from screens that were never navigated to.
//  · A one-frame light flash at the handover, from the gap between one instance unmounting and the
//    next painting.
//
// Each of those was fixable in isolation and the next one appeared. They share a cause: a screen
// rendered outside the tab that owns it. A faithful interactive transition needs the platform to own
// it — either a horizontal `ScrollView` with `pagingEnabled`, whose UIScrollView cancels touches in
// its subviews the moment it starts scrolling (the mechanism RN's Pressable does not give us here),
// or an interactive UITabBarController transition in Swift. Both are real work and neither should be
// shipped blind again.
//
// What survives: the gesture itself, its thresholds (see lib/tabGesture.ts — a thumb swipe arcs, so
// the pager must cancel later vertically than it activates horizontally), and no haptic on arrival,
// because tapping a native tab bar does not buzz either.
import { router } from "expo-router";
import React from "react";
import { StyleSheet, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";

import { isNative } from "../lib/platform";
import { ACTIVATE_X, COMMIT_DISTANCE, COMMIT_VELOCITY, FAIL_Y } from "../lib/tabGesture";
import { tabHrefAt, TAB_SWIPE_ORDER } from "../lib/tabSwipe";

export function TabPager({ index, children }: { index: number; children: React.ReactNode }) {
  const hasPrev = index > 0;
  const hasNext = index < TAB_SWIPE_ORDER.length - 1;

  const pan = React.useMemo(
    () =>
      Gesture.Pan()
        .enabled(isNative)
        .activeOffsetX([-ACTIVATE_X, ACTIVATE_X])
        .failOffsetY([-FAIL_Y, FAIL_Y])
        // Nothing here animates a shared value, so the whole gesture runs on the JS thread and can
        // call the router directly.
        .runOnJS(true)
        .onEnd((event) => {
          const committed =
            Math.abs(event.translationX) > COMMIT_DISTANCE ||
            Math.abs(event.velocityX) > COMMIT_VELOCITY;
          if (!committed) return;
          const toPrev = event.translationX > 0;
          if (toPrev ? !hasPrev : !hasNext) return;
          const href = tabHrefAt(toPrev ? index - 1 : index + 1);
          if (href !== null) router.navigate(href);
        }),
    [hasNext, hasPrev, index],
  );

  return (
    <GestureDetector gesture={pan}>
      <View style={styles.fill}>{children}</View>
    </GestureDetector>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
});
