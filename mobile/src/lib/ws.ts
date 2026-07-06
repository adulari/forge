// WebSocket client for the Forge daemon session protocol v7. See BUILD_PLAN.md §1.3.
//
// One `Snapshot` frame per server message (full state, not a delta). Client tracks the
// last-seen `revision` and reconnects with `?rev=<revision>` for replay; `resync:true`
// frames are accepted even though `revision` jumps. `closed:true` stops all reconnects.

import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, type AppStateStatus } from "react-native";

export const PROTOCOL_VERSION = 7;

// ---------------------------------------------------------------------------
// Wire types (verbatim field names, §1.3)
// ---------------------------------------------------------------------------

export interface SnapshotTask {
  title: string;
  status: "pending" | "in_progress" | "done";
}

export interface SnapshotSubagent {
  agent: string;
  task: string;
  model: string | null;
  last: string;
  done: boolean;
  cost: number;
}

export interface OverlayRow {
  id: string;
  label: string;
  detail: string;
  selected: boolean;
  group: string | null;
}

export interface Overlay {
  kind: string; // "palette" | "picker:<k>" | "config" | "overlay:usage" | "overlay:mesh" | "overlay:workflow"
  title: string;
  rows: OverlayRow[];
  selected: number;
  filter: string | null;
  free_text: boolean;
  body: string | null;
}

export interface DiffHunk {
  header: string;
  lines: string[]; // first char is the gutter: "+" | "-" | " "
}

export interface DiffFile {
  path: string;
  kind: "created" | "modified" | "deleted";
  binary: boolean;
  adds: number;
  dels: number;
  hunks: DiffHunk[];
  skipped_lines: number;
}

export interface Diff {
  pending: boolean;
  skipped_files: number;
  files: DiffFile[];
}

export interface PlanStep {
  title: string;
  detail: string;
}

export interface Plan {
  title: string;
  steps: PlanStep[];
  notes: string | null;
}

export interface QuestionOption {
  label: string;
  description: string;
}

export interface Snapshot {
  protocol: number;
  session_id: string;
  title: string;
  cwd: string;
  worktree: string | null;
  exposure: string; // "loopback" | "LAN" | "public (…)"
  busy: boolean;
  done: boolean;
  temper: string;
  tier: string | null;
  model: string;
  cost_usd: number;
  context_tokens: number;
  context_limit: number | null;
  streaming: string;
  transcript: string[];
  tasks: SnapshotTask[];
  subagents: SnapshotSubagent[];
  queued: string[];
  permission_prompt: string | null;
  question: string | null;
  question_options: QuestionOption[];
  question_allow_other: boolean;
  overlay: Overlay | null;
  diff: Diff | null;
  plan: Plan | null;
  copy_text: string | null;
  prompt_seq: number;
  notes: string[];
  revision: number;
  resync: boolean;
  closed: boolean;
}

export type RemoteInput =
  | { kind: "prompt"; text: string }
  | { kind: "allow"; yes: boolean; seq: number }
  | { kind: "answer"; text: string; seq: number }
  | { kind: "interrupt" }
  | { kind: "key"; key: string }
  | { kind: "overlay_select"; id: string }
  | { kind: "overlay_nav"; delta: number }
  | { kind: "overlay_filter"; text: string }
  | { kind: "overlay_cancel" };

export type ConnectionState =
  | "idle"
  | "connecting"
  | "open"
  | "reconnecting"
  | "closed";

const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15000];

function wsUrl(baseUrl: string, sessionId: string, rev: number): string {
  const u = new URL(`${baseUrl}/ws`);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.searchParams.set("session", sessionId);
  u.searchParams.set("rev", String(rev));
  return u.toString();
}

export interface UseSessionSocketResult {
  snapshot: Snapshot | null;
  connectionState: ConnectionState;
  send: (input: RemoteInput) => void;
}

/**
 * Connects `/ws?session&rev`, exposes the latest Snapshot, and a `send` for RemoteInput.
 * Auto-reconnects with backoff + rev replay; pauses on app background, resumes on
 * foreground (UI_RULES.md #20).
 */
export function useSessionSocket(
  baseUrl: string | null,
  sessionId: string | null,
): UseSessionSocketResult {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>("idle");

  const wsRef = useRef<WebSocket | null>(null);
  const revRef = useRef(0);
  const attemptRef = useRef(0);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const closedRef = useRef(false);
  const appStateRef = useRef<AppStateStatus>(AppState.currentState);
  const shouldRunRef = useRef(true);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current != null) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const teardown = useCallback(() => {
    clearReconnectTimer();
    if (wsRef.current) {
      const ws = wsRef.current;
      wsRef.current = null;
      ws.onopen = null;
      ws.onmessage = null;
      ws.onerror = null;
      ws.onclose = null;
      try {
        ws.close();
      } catch {
        // already closed
      }
    }
  }, [clearReconnectTimer]);

  const connect = useCallback(() => {
    if (!baseUrl || !sessionId || closedRef.current || !shouldRunRef.current) return;
    teardown();
    setConnectionState((s) => (s === "idle" ? "connecting" : "reconnecting"));

    const ws = new WebSocket(wsUrl(baseUrl, sessionId, revRef.current));
    wsRef.current = ws;

    ws.onopen = () => {
      attemptRef.current = 0;
      setConnectionState("open");
    };

    ws.onmessage = (event) => {
      let data: Snapshot;
      try {
        data = JSON.parse(String(event.data));
      } catch {
        return;
      }
      if (data.revision != null) {
        // Dedupe on revision, but always accept resync frames (revision may jump).
        if (!data.resync && data.revision <= revRef.current && revRef.current !== 0) {
          return;
        }
        revRef.current = data.revision;
      }
      setSnapshot(data);
      if (data.closed) {
        closedRef.current = true;
        setConnectionState("closed");
        teardown();
      }
    };

    ws.onerror = () => {
      // onclose follows; reconnect scheduled there.
    };

    ws.onclose = () => {
      wsRef.current = null;
      if (closedRef.current || !shouldRunRef.current) return;
      setConnectionState("reconnecting");
      const delay =
        BACKOFF_MS[Math.min(attemptRef.current, BACKOFF_MS.length - 1)];
      attemptRef.current += 1;
      clearReconnectTimer();
      reconnectTimerRef.current = setTimeout(connect, delay);
    };
  }, [baseUrl, sessionId, teardown, clearReconnectTimer]);

  useEffect(() => {
    closedRef.current = false;
    revRef.current = 0;
    attemptRef.current = 0;
    shouldRunRef.current = true;
    setSnapshot(null);
    setConnectionState("idle");
    connect();
    return () => {
      shouldRunRef.current = false;
      teardown();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseUrl, sessionId]);

  useEffect(() => {
    const sub = AppState.addEventListener("change", (next: AppStateStatus) => {
      const prev = appStateRef.current;
      appStateRef.current = next;
      if (next.match(/inactive|background/)) {
        shouldRunRef.current = false;
        teardown();
        setConnectionState("idle");
      } else if (prev.match(/inactive|background/) && next === "active") {
        shouldRunRef.current = true;
        connect();
      }
    });
    return () => sub.remove();
  }, [connect, teardown]);

  const send = useCallback((input: RemoteInput) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(input));
    }
  }, []);

  return { snapshot, connectionState, send };
}
