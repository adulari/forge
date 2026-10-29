import { useCallback, useEffect, useRef, useState } from "react";

import { listTerminalSessions, type TerminalSessionSummary } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import {
  advanceActivity,
  EMPTY_TERMINAL_ACTIVITY,
  hasUnseenOutput,
  type TerminalActivity,
} from "./terminalModel";

const POLL_MS = 2_500;

export interface TerminalActivityView {
  summaries: TerminalSessionSummary[];
  activity: TerminalActivity;
  /** A terminal in the background that printed since it was last looked at. */
  isUnseen: (terminalId: string) => boolean;
  isStreaming: (terminalId: string) => boolean;
  exitCodeOf: (terminalId: string) => number | null | undefined;
  refresh: () => Promise<void>;
}

/**
 * Follows a session's terminals from the daemon's output counters, so a tab nobody is attached
 * to can still say it has printed something. Polls only while `enabled`.
 */
export function useTerminalActivity(
  sessionId: string | null,
  activeId: string | null,
  enabled: boolean,
): TerminalActivityView {
  const { baseUrl } = useAuth();
  const [summaries, setSummaries] = useState<TerminalSessionSummary[]>([]);
  const [activity, setActivity] = useState<TerminalActivity>(EMPTY_TERMINAL_ACTIVITY);
  const activeRef = useRef(activeId);
  useEffect(() => {
    activeRef.current = activeId;
  }, [activeId]);
  const sessionRef = useRef(sessionId);

  const refresh = useCallback(async () => {
    if (!baseUrl || !sessionId) return;
    try {
      const next = await listTerminalSessions(baseUrl, sessionId);
      if (sessionRef.current !== sessionId) return;
      setSummaries(next);
      setActivity((previous) => advanceActivity(previous, next, activeRef.current));
    } catch {
      // The tabs keep their last known state while the daemon is unreachable.
    }
  }, [baseUrl, sessionId]);

  useEffect(() => {
    sessionRef.current = sessionId;
    setSummaries([]);
    setActivity(EMPTY_TERMINAL_ACTIVITY);
  }, [sessionId]);

  useEffect(() => {
    if (!enabled) return;
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  }, [enabled, refresh, activeId]);

  const isUnseen = useCallback(
    (terminalId: string) => hasUnseenOutput(activity, terminalId, activeId),
    [activity, activeId],
  );
  const isStreaming = useCallback(
    (terminalId: string) => terminalId !== activeId && activity.streaming[terminalId] === true,
    [activity, activeId],
  );
  const exitCodeOf = useCallback(
    (terminalId: string) => {
      const summary = summaries.find((candidate) => candidate.terminal_id === terminalId);
      return summary?.status === "exited" ? (summary.exit_code ?? null) : undefined;
    },
    [summaries],
  );

  return { summaries, activity, isUnseen, isStreaming, exitCodeOf, refresh };
}
