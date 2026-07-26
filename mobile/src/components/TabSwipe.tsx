// Horizontal swipe anywhere on a tab screen moves to the neighbouring tab, so the bottom bar is no
// longer the only way across. Wraps the tabs navigator in `(tabs)/_layout.tsx`, which scopes it to
// exactly the four tab screens: /floor, /plans and /session/[id] live in the ROOT stack (see that
// file's invariant comment), so a swipe on a pushed screen cannot switch tabs.
//
// Three conflicts this has to lose gracefully, in order of how badly they'd break:
//
//  1. SessionCard's own swipe-to-archive pan on Fleet, which activates at 10px. This gesture waits
//     for 28px, and react-native-gesture-handler cancels an ancestor once a descendant activates —
//     so the card claims the drag first and the tab never moves under it. The thresholds are what
//     encode that, and they are the reason not to lower them.
//  2. Vertical scrolling on every tab. `failOffsetY` drops this gesture the moment the finger moves
//     15px vertically, so a scroll that drifts sideways never lands on another tab.
//  3. Horizontally scrollable rows inside screens (Segmented, filter chips). They are descendants,
//     so case 1's rule covers them too.
//
// Deliberately native-only: on web and the Tauri desktop webview a click-drag is a text selection,
// and a trackpad swipe arrives as wheel deltas rather than a pan, so there is nothing to hook.
import { router, usePathname } from "expo-router";
import React from "react";
import { StyleSheet, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";

import { haptics } from "../lib/haptics";
import { isNative } from "../lib/platform";
import { nextTabHref } from "../lib/tabSwipe";

/** Horizontal distance before this gesture takes over — comfortably past SessionCard's 10px. */
const ACTIVATE_X = 28;
/** Vertical slop that hands the drag back to a scroll view. */
const FAIL_Y = 15;
/** A committed swipe: either far enough, or fast enough to read as a flick. */
const COMMIT_DISTANCE = 64;
const COMMIT_VELOCITY = 420;

export function TabSwipe({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  const pan = React.useMemo(
    () =>
      Gesture.Pan()
        .enabled(isNative)
        .activeOffsetX([-ACTIVATE_X, ACTIVATE_X])
        .failOffsetY([-FAIL_Y, FAIL_Y])
        // The handler navigates and fires haptics, both JS-thread APIs. Running the whole gesture on
        // JS is simpler — and correct — where no value is animated from the UI thread.
        .runOnJS(true)
        .onEnd((event) => {
          const committed =
            Math.abs(event.translationX) > COMMIT_DISTANCE || Math.abs(event.velocityX) > COMMIT_VELOCITY;
          if (!committed) return;
          const href = nextTabHref(pathname, event.translationX < 0 ? "left" : "right");
          if (href === null) return;
          haptics.select();
          router.navigate(href);
        }),
    [pathname],
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
