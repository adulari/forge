// Active session segment (Chat/Tasks/...). On web/desktop a switch is a LOCAL state change that
// paints immediately; the URL follows from an idle callback so deep links, reload and Back still
// see the segment. A router navigation re-renders the whole shell (root navigator, sidebar,
// palette, session layout) and was the largest part of a tab switch once the segments stayed
// mounted. Native keeps the plain `router.replace`.
import { router } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";

import { segmentFromPathname, type SegmentValue } from "./sessionSegments";

const SUFFIX: Record<SegmentValue, string> = {
  chat: "",
  tasks: "tasks",
  agents: "agents",
  review: "review",
  files: "files",
  terminal: "terminal",
  replay: "replay",
};

export function segmentHref(sessionId: string, segment: SegmentValue): string {
  const suffix = SUFFIX[segment];
  return `/session/${sessionId}${suffix ? `/${suffix}` : ""}`;
}

type IdleHandle = { kind: "idle"; id: number } | { kind: "timeout"; id: ReturnType<typeof setTimeout> };

function scheduleIdle(fn: () => void): IdleHandle {
  const w = typeof window === "undefined" ? undefined : (window as Window & { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number });
  if (w?.requestIdleCallback) return { kind: "idle", id: w.requestIdleCallback(fn, { timeout: 400 }) };
  return { kind: "timeout", id: setTimeout(fn, 120) };
}

function cancelIdle(handle: IdleHandle | null): void {
  if (!handle) return;
  if (handle.kind === "idle") (window as Window & { cancelIdleCallback?: (id: number) => void }).cancelIdleCallback?.(handle.id);
  else clearTimeout(handle.id);
}

export function useSessionSegment(
  pathname: string,
  sessionId: string,
  local: boolean,
): { segment: SegmentValue; setSegment: (next: SegmentValue) => void } {
  const routed = segmentFromPathname(pathname);
  const [override, setOverride] = useState<SegmentValue | null>(null);
  const pending = useRef<SegmentValue | null>(null);
  const handle = useRef<IdleHandle | null>(null);
  const pathnameRef = useRef(pathname);

  // The URL caught up with the override, or something else navigated: drop the override.
  useEffect(() => {
    if (override == null) return;
    if (routed === override) {
      pending.current = null;
      setOverride(null);
    } else if (pending.current == null) {
      setOverride(null);
    }
  }, [routed, override, pathname]);

  useEffect(() => () => cancelIdle(handle.current), []);

  const segment = override ?? routed;
  const segmentRef = useRef(segment);
  useEffect(() => {
    pathnameRef.current = pathname;
    segmentRef.current = segment;
  });

  const setSegment = useCallback(
    (next: SegmentValue) => {
      if (next === segmentRef.current) return;
      if (!local) {
        router.replace(segmentHref(sessionId, next) as never);
        return;
      }
      pending.current = next;
      setOverride(next);
      cancelIdle(handle.current);
      handle.current = scheduleIdle(() => {
        const target = pending.current;
        if (target != null && segmentFromPathname(pathnameRef.current) !== target) {
          router.replace(segmentHref(sessionId, target) as never);
        } else {
          pending.current = null;
        }
      });
    },
    [local, sessionId],
  );

  return { segment, setSegment };
}
