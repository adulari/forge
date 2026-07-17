// WebSocket client for the Forge daemon session protocol v8. See protocol/remote-v8.json.
//
// One `Snapshot` frame per server message (full state, not a delta). Client tracks the
// last-seen `revision` and reconnects with `?rev=<revision>` for replay; `resync:true`
// frames are accepted even though `revision` jumps. `closed:true` stops all reconnects.

import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, type AppStateStatus, Platform } from "react-native";

import { TWebSocket } from "./transport";
import { decideSnapshotRevision } from "./sessionReconciler";
import { isValidSnapshotFrame as isValidSnapshotIdentity } from "./remoteProtocol";
export { PROTOCOL_VERSION } from "./remoteProtocol";

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
  project_initialized: boolean;
  project_init_hint: string | null;
  exposure: string; // "loopback" | "LAN" | "public (…)"
  busy: boolean;
  done: boolean;
  temper: string;
  effort?: string | null;
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
  /** AI-suggested likely next user prompt, refreshed after each completed turn. Absent/null
   * while none is available — never implies one is pending. */
  suggested_prompt?: string | null;
  prompt_seq: number;
  notes: string[];
  revision: number;
  resync: boolean;
  closed: boolean;
}

/** Protocol identity guard narrowed to the full snapshot contract used by this adapter. */
export function isValidSnapshotFrame(value: unknown): value is Snapshot {
  return isValidSnapshotIdentity(value);
}

export type RemoteInput =
  | {
      kind: "prompt";
      text: string;
      /** Server-relative upload paths this specific prompt carries (Composer's `SentAttachment`s
       * with a `path`) — correlates an attachment to THIS send so it can't leak into an
       * unrelated adjacent prompt. Omitted (not just empty) by any caller that predates this
       * field; the server treats a missing/empty list as "fall back to the old ambient
       * upload-then-prompt sequence". */
      attachments?: { path: string; image: boolean }[];
    }
  | { kind: "allow"; yes: boolean; seq: number }
  | { kind: "answer"; text: string; seq: number }
  | { kind: "interrupt" }
  // Cancels one server-queued prompt; index+text echo Snapshot.queued so a shifted queue is
  // detected server-side and dropped as stale.
  | { kind: "dequeue"; index: number; text: string }
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
  | "unreachable"
  | "closed";

const BACKOFF_MS = [500, 1000, 2000, 4000, 8000, 15000];
// After this many consecutive failed attempts (~15s of backoff), stop presenting the
// outage as a routine "reconnecting…" blip and escalate to a harder "unreachable"
// state — retries keep running in the background, but the UI should say so plainly
// instead of leaving the user staring at a soft, indefinite "reconnecting…" forever.
const UNREACHABLE_AFTER_ATTEMPTS = 5;
const LIVENESS_TIMEOUT_MS = 35_000;

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
  send: (input: RemoteInput) => boolean;
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
  // Set when a revision gap forced the current socket closed (see onmessage). A gap is a replay
  // artifact, not a connection failure, so onclose reconnects immediately at attempt 0 without
  // escalating backoff or the unreachable counter.
  const resyncPendingRef = useRef(false);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const socketGenerationRef = useRef(0);
  const closedRef = useRef(false);
  const appStateRef = useRef<AppStateStatus>(AppState.currentState);
  const shouldRunRef = useRef(true);
  const livenessTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current != null) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const clearLivenessTimer = useCallback(() => {
    if (livenessTimerRef.current != null) {
      clearTimeout(livenessTimerRef.current);
      livenessTimerRef.current = null;
    }
  }, []);

  const teardown = useCallback(() => {
    clearReconnectTimer();
    clearLivenessTimer();
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
  }, [clearReconnectTimer, clearLivenessTimer]);

  const connect = useCallback(() => {
    if (!baseUrl || !sessionId || closedRef.current || !shouldRunRef.current) return;
    teardown();
    setConnectionState((s) =>
      s === "idle" ? "connecting" : s === "unreachable" ? "unreachable" : "reconnecting",
    );

    const generation = ++socketGenerationRef.current;
    const ws = new TWebSocket(wsUrl(baseUrl, sessionId, revRef.current));
    wsRef.current = ws;

    const armLivenessWatchdog = () => {
      clearLivenessTimer();
      if (appStateRef.current.match(/background|inactive/)) return;
      livenessTimerRef.current = setTimeout(() => {
        livenessTimerRef.current = null;
        if (generation !== socketGenerationRef.current || !shouldRunRef.current) return;
        // A half-open socket never emits close/error, so closing it enters the normal
        // generation-guarded onclose/backoff path below.
        try {
          ws.close();
        } catch {
          // already closed
        }
      }, LIVENESS_TIMEOUT_MS);
    };

    ws.onopen = () => {
      if (generation !== socketGenerationRef.current) return;
      attemptRef.current = 0;
      setConnectionState("open");
      armLivenessWatchdog();
    };

    ws.onmessage = (event) => {
      if (generation !== socketGenerationRef.current) return;
      armLivenessWatchdog();
      let data: unknown;
      try {
        data = JSON.parse(String(event.data));
      } catch {
        console.warn("[ws] dropped malformed snapshot frame");
        return;
      }
      // Keepalive heartbeats are expected wire traffic, not malformed snapshots — ack
      // liveness silently instead of warning on every one.
      if (data != null && typeof data === "object" && "keepalive" in (data as object)) {
        return;
      }
      if (!isValidSnapshotFrame(data)) {
        console.warn("[ws] dropped invalid snapshot frame");
        return;
      }
      if (data.revision != null) {
        const revisionDecision = decideSnapshotRevision(revRef.current, data);
        if (revisionDecision === "replay") {
          // A non-contiguous frame means replay/watch coalescing skipped state. Reconnect
          // from the last known-good revision so the server can replay or explicitly resync.
          // This is a transient gap, not a failed connection — flag it so onclose reconnects
          // at once without counting toward the backoff/unreachable escalation.
          resyncPendingRef.current = true;
          ws.close();
          return;
        }
        // Dedupe on revision, but always accept resync frames (revision may jump).
        if (revisionDecision === "duplicate") return;
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
      if (generation !== socketGenerationRef.current) return;
      wsRef.current = null;
      if (closedRef.current || !shouldRunRef.current) return;
      if (resyncPendingRef.current) {
        // A revision gap forced this close, not a real outage. Reconnect immediately from the
        // last good revision without incrementing attemptRef, so a transient replay gap never
        // imposes backoff or falsely escalates to "unreachable".
        resyncPendingRef.current = false;
        setConnectionState("reconnecting");
        clearReconnectTimer();
        reconnectTimerRef.current = setTimeout(connect, 0);
        return;
      }
      setConnectionState(attemptRef.current >= UNREACHABLE_AFTER_ATTEMPTS ? "unreachable" : "reconnecting");
      const delay =
        BACKOFF_MS[Math.min(attemptRef.current, BACKOFF_MS.length - 1)];
      attemptRef.current += 1;
      clearReconnectTimer();
      reconnectTimerRef.current = setTimeout(connect, delay);
    };
  }, [baseUrl, sessionId, teardown, clearReconnectTimer, clearLivenessTimer]);

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
    const goBackground = () => {
      shouldRunRef.current = false;
      teardown();
      setConnectionState("idle");
    };
    const goForeground = () => {
      shouldRunRef.current = true;
      connect();
    };

    // Web has no AppState lifecycle — branch on the DOM's visibilitychange directly.
    if (Platform.OS === "web") {
      const wasHidden = { current: document.visibilityState === "hidden" };
      const onVisibilityChange = () => {
        const isHidden = document.visibilityState === "hidden";
        if (isHidden && !wasHidden.current) {
          goBackground();
        } else if (!isHidden && wasHidden.current) {
          goForeground();
        }
        wasHidden.current = isHidden;
      };
      document.addEventListener("visibilitychange", onVisibilityChange);
      return () => document.removeEventListener("visibilitychange", onVisibilityChange);
    }

    const sub = AppState.addEventListener("change", (next: AppStateStatus) => {
      const prev = appStateRef.current;
      appStateRef.current = next;
      if (next.match(/inactive|background/)) {
        goBackground();
      } else if (prev.match(/inactive|background/) && next === "active") {
        goForeground();
      }
    });
    return () => sub.remove();
  }, [connect, teardown]);

  const send = useCallback((input: RemoteInput): boolean => {
    const ws = wsRef.current;
    if (ws && ws.readyState === TWebSocket.OPEN) {
      try {
        ws.send(JSON.stringify(input));
        return true;
      } catch {
        ws.close();
      }
    }
    return false;
  }, []);

  return { snapshot, connectionState, send };
}
