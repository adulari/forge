// The two docks that keep the transcript honest: why the last turn failed (the host's structured
// ThreadError) and that the history on screen is known to be incomplete, either because the host
// could not replay what this device missed while away or because its stored record has holes.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, History } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";

import { getHistoryIntegrity } from "../../lib/api";
import { gapAckKey, liveGapNotice, missingRowsNotice, needsAck, type GapNotice } from "../../lib/historyGap";
import { hasMoreDetail, threadErrorKey, threadErrorTitle } from "../../lib/threadError";
import type { Snapshot } from "../../lib/ws";
import { NoticeDock } from "./NoticeDock";

export interface IntegrityDocksProps {
  baseUrl: string | null;
  sessionId: string;
  snapshot: Snapshot | null;
}

function useStoredGapNotice(baseUrl: string | null, sessionId: string, refreshKey: number): {
  notice: GapNotice | null;
  acknowledge: () => void;
} {
  const query = useQuery({
    queryKey: ["historyIntegrity", baseUrl, sessionId, refreshKey],
    queryFn: () => getHistoryIntegrity(baseUrl as string, sessionId),
    enabled: baseUrl != null,
    staleTime: 60_000,
    retry: false,
  });
  const [acked, setAcked] = useState<number | null>(null);
  const ackKey = baseUrl ? gapAckKey(baseUrl, sessionId) : null;
  useEffect(() => {
    setAcked(null);
    if (!ackKey) return;
    let live = true;
    AsyncStorage.getItem(ackKey)
      .then((raw) => {
        if (live) setAcked(raw == null ? 0 : Number(raw) || 0);
      })
      .catch(() => {
        if (live) setAcked(0);
      });
    return () => {
      live = false;
    };
  }, [ackKey]);
  const gap = query.data ?? null;
  const notice = acked != null && needsAck(gap, acked) && gap ? missingRowsNotice(gap) : null;
  const acknowledge = () => {
    if (!gap || !ackKey) return;
    setAcked(gap.count);
    AsyncStorage.setItem(ackKey, String(gap.count)).catch(() => {
      // Acknowledged for this screen at least; it may show again next launch.
    });
  };
  return { notice, acknowledge };
}

export function IntegrityDocks({ baseUrl, sessionId, snapshot }: IntegrityDocksProps) {
  const threadError = snapshot?.thread_error ?? null;
  const [dismissedError, setDismissedError] = useState<string | null>(null);
  const errorKey = threadError ? threadErrorKey(threadError) : null;
  useEffect(() => {
    if (errorKey == null) setDismissedError(null);
  }, [errorKey]);

  const liveGap = snapshot?.history_gap ?? null;
  const liveRevision = snapshot?.revision ?? 0;
  const [live, setLive] = useState<{ notice: GapNotice; session: string } | null>(null);
  useEffect(() => {
    if (!liveGap) return;
    const notice = liveGapNotice(liveGap, liveRevision);
    setLive((current) => (current?.session === sessionId && current.notice.id === notice.id ? current : { notice, session: sessionId }));
  }, [liveGap, liveRevision, sessionId]);
  const liveNotice = live?.session === sessionId ? live.notice : null;

  const stored = useStoredGapNotice(baseUrl, sessionId, liveNotice ? 1 : 0);
  const notices = useMemo(() => [liveNotice, stored.notice].filter((n): n is GapNotice => n != null), [liveNotice, stored.notice]);

  return (
    <>
      {threadError && errorKey !== dismissedError ? (
        <NoticeDock
          tone="danger"
          icon={AlertTriangle}
          title={threadErrorTitle(threadError)}
          summary={threadError.summary}
          details={hasMoreDetail(threadError) ? threadError.details : undefined}
          onDismiss={() => setDismissedError(errorKey)}
          dismissLabel="Dismiss error"
          testID="thread-error-dock"
        />
      ) : null}
      {notices.map((notice) => (
        <NoticeDock
          key={notice.id}
          tone="warn"
          icon={History}
          title={notice.title}
          summary="Some of this session may be missing."
          details={notice.detail}
          onDismiss={() => (notice === liveNotice ? setLive(null) : stored.acknowledge())}
          dismissLabel="Dismiss history notice"
          testID="history-gap-dock"
        />
      ))}
    </>
  );
}
