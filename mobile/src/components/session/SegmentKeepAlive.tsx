// Desktop/web session segments (Chat/Tasks/Agents/Review/Files/Replay) stay mounted once opened.
//
// Before: every segment switch was `router.replace` into a `<Slot/>` keyed by segment, so going
// Chat -> Tasks -> Chat tore down the whole transcript + composer and rebuilt it (and replayed a
// 6px/150ms fade on top) — ~170 ms of JS and layout per return trip on a 70-turn session.
// Now the session layout renders the visited segments itself, each inside React's <Activity>:
// the active one is shown, the others are `display: none` with their state, DOM and scroll
// position kept, their effects paused and their re-renders deferred to idle time. The route
// files still exist (deep links, `router.replace`, back/forward all keep working); on desktop
// their default exports render nothing and the layout owns the screen via `SEGMENT_SCREENS`.
//
// Phones keep the old `<Slot/>` behaviour untouched: a mounted-but-hidden native screen would
// hold memory the OS reclaims today, so `KEEP_SEGMENTS_ALIVE` is web-only.
import React, { Activity, createContext, useContext, startTransition, useEffect, useState } from "react";
import { Platform, StyleSheet, View } from "react-native";

export type KeptSegment = "chat" | "tasks" | "agents" | "review" | "files" | "replay";

export const KEEP_SEGMENTS_ALIVE = Platform.OS === "web";

/** Set by the session layout when it renders the segments itself. */
export const SegmentsHostedContext = createContext(false);

/** True where the session host renders the pushed workflow screens itself (the multi-session
 * desktop host has no per-session route outlet). */
export const RouteScreensHostedContext = createContext(false);

export function useRouteScreensHosted(): boolean {
  return useContext(RouteScreensHostedContext);
}

/** Route files call this: true means the layout already renders this segment. */
export function useSegmentsHosted(): boolean {
  return useContext(SegmentsHostedContext);
}

export interface SegmentScreen {
  segment: KeptSegment;
  Component: React.ComponentType;
}

// Memoised: the layout re-renders on every WS snapshot frame, but with a module-level `screens`
// array this only re-renders when the active segment changes. Each screen subscribes to the
// session context itself for what it needs.
export const SegmentKeepAlive = React.memo(function SegmentKeepAlive({
  active,
  screens,
  deferFirstFrame = true,
}: {
  /** Paint the header before the body. False for a session mounted hidden (prewarm): its effects
   * do not run, so a deferral waiting on one would never end. */
  deferFirstFrame?: boolean;
  /** The segment to show, or null while a non-segment route (workflow, ...) is on top. */
  active: KeptSegment | null;
  screens: readonly SegmentScreen[];
}) {
  const [visited, setVisited] = useState<ReadonlySet<KeptSegment>>(() => new Set(active ? [active] : []));
  if (active && !visited.has(active)) {
    setVisited(new Set([...visited, active]));
  }

  // Lightweight first frame: a session that has just been opened paints its header and tab strip
  // (rendered by the layout around this host) in the first commit; the segments, the transcript
  // above all, mount in the next frame. Without it the open waits for the whole screen.
  const [bodyReady, setBodyReady] = useState(!deferFirstFrame);
  useEffect(() => {
    if (bodyReady) return;
    let raf = 0;
    const timer = setTimeout(() => {
      raf = requestAnimationFrame(() => startTransition(() => setBodyReady(true)));
    }, 0);
    return () => {
      clearTimeout(timer);
      cancelAnimationFrame(raf);
    };
  }, [bodyReady]);

  // Prewarm: once the session has painted, mount the remaining segments one idle slice at a time
  // (hidden, so their effects and data fetches stay paused) so the FIRST visit to a tab is a
  // display toggle too, not a mount.
  const hasActive = active != null;
  const remaining = screens.find((screen) => !visited.has(screen.segment) && screen.segment !== active);
  useEffect(() => {
    if (!hasActive || !remaining) return;
    const w = window as Window & { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number; cancelIdleCallback?: (id: number) => void };
    let idleId: number | undefined;
    const timer = setTimeout(() => {
      const warm = () => setVisited((prev) => (prev.has(remaining.segment) ? prev : new Set([...prev, remaining.segment])));
      if (w.requestIdleCallback) idleId = w.requestIdleCallback(warm, { timeout: 1000 });
      else warm();
    }, 150);
    return () => {
      clearTimeout(timer);
      if (idleId != null) w.cancelIdleCallback?.(idleId);
    };
  }, [hasActive, remaining]);
  if (!bodyReady) return <View style={styles.shown} />;
  return (
    <>
      {screens
        .filter((screen) => screen.segment === active || visited.has(screen.segment))
        .map((screen) => {
          const shown = screen.segment === active;
          return (
            <Activity key={screen.segment} mode={shown ? "visible" : "hidden"}>
              <View style={shown ? styles.shown : styles.hidden}>
                <screen.Component />
              </View>
            </Activity>
          );
        })}
    </>
  );
});

const styles = StyleSheet.create({
  shown: { flex: 1 },
  hidden: { display: "none" },
});
