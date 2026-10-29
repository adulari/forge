// WebSocket client for the Forge daemon session protocol v10. See protocol/remote-v10.json.
//
// One `Snapshot` frame per server message (full state, not a delta). Client tracks the
// last-seen `revision` and reconnects with `?rev=<revision>` for replay; `resync:true`
// frames are accepted even though `revision` jumps. `closed:true` stops all reconnects.

import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, type AppStateStatus, Platform } from "react-native";

import { createFrameCoalescer } from "./frameCoalescer";
import { accumulateStreaming, EMPTY_STREAMING, streamingDisplayText, type StreamingAccumulator } from "./streamingText";
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
  /** v9 additive, reserved: the task machinery (`update_tasks`) tracks no owner, so the host
   * always sends null. Render tasks as unassigned — never infer an owner from this. */
  assignee?: string | null;
}

/** v9 additive: where one transcript line came from, so a tool result can render as a block
 * instead of being guessed at from its text.
 *
 * `"nudge"` mirrors `lib/api.ts`'s `TranscriptKind` for a shared vocabulary across both wires,
 * but the live socket does not actually emit it today — a harness-injected continuation nudge
 * isn't echoed into the live scrollback, only persisted (see `/api/history`'s `kind`). Kept here
 * so `toHistoryRow` (transcriptFiller.ts) never has to narrow between the two. */
export type TranscriptKind = "user" | "assistant" | "tool" | "system" | "nudge" | "job_notice" | "harness_notice";

/** v9 additive: `Snapshot.transcript` with provenance — same lines, same order, one row per
 * entry. Empty from a pre-v9 host, which is why `transcript` remains the fallback. */
export interface TranscriptRow {
  kind: TranscriptKind;
  text: string;
  /** Tool name on `kind: "tool"` rows. */
  tool?: string | null;
  /** "ok" | "failed" on a tool RESULT row; null on the call row that precedes it. */
  meta?: string | null;
}

export interface SnapshotSubagent {
  // v8.1 additive: `id` (stable row key), `phase` (workflow phase name, null for plain
  // subagents), `ok` (last-turn success). Server serializes Option<T> as T|null.
  id: string;
  agent: string;
  task: string;
  model: string | null;
  phase: string | null;
  last: string;
  done: boolean;
  ok: boolean;
  cost: number;
  /** v9 additive, reserved: subagents run headless (a permission `Ask` resolves as Deny inside
   * a child), so no child is ever blocked on its own prompt and the host always sends null.
   * Session-level prompts live on `Snapshot.permission_prompt`. */
  permission_prompt?: string | null;
  /** The child's own persisted session id, when the host can serve it as a nested read-only
   * session (`/ws?session=<id>`, `/api/history`). Absent from older hosts and for subagents that
   * have no session of their own — those rows stay non-interactive. */
  session_id?: string | null;
}

/** v8.1 additive: workflow-run state. Absent/null when no workflow is active. Server
 * serializes Option<T> as T|null. Workflow agent rows are the `Snapshot.subagents` entries
 * whose `phase` is set. */
export interface SnapshotWorkflow {
  active: boolean;
  name: string | null;
  phases: string[];
  logs: string[];
  finished_ok: boolean | null;
  summary: string | null;
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
  /** v9 additive: the status of the task an approved plan seeded from this step (approval is
   * what gives a step execution state at all). "queued" whenever no task carries the step's
   * title — an unapproved proposal, or a task list the model has rewritten. Absent from a
   * pre-v9 host, which reads the same as "queued". */
  status?: "queued" | "in_progress" | "done";
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

/** One question of an `ask_user` form (v11 additive; see `Snapshot.question_form`). */
export interface FormQuestion {
  header: string;
  text: string;
  options: QuestionOption[];
  multi: boolean;
  allow_other: boolean;
  allow_note: boolean;
}

/** The answer to one `FormQuestion`, sent back as `answer{text: JSON.stringify({answers})}`. */
export interface FormAnswer {
  selected: string[];
  other?: string | null;
  note?: string | null;
}

export type GoalPhase = "active" | "paused" | "budget_limit" | "complete" | "failed" | "cancelled";

/** Additive: the `/goal` run as the host tracks it. Absent from an older host and before any goal
 * starts; a client then falls back to the transient notes (see `lib/goalState.ts`). */
export interface SnapshotGoal {
  objective: string;
  state: GoalPhase;
  iteration: number;
  tokens_used: number;
  max_tokens: number | null;
  max_minutes: number | null;
  /** Run time as of `synced_at`; only an `active` goal keeps running past it. */
  elapsed_secs: number;
  /** Unix ms of the last update to the counters above. */
  synced_at: number;
  gates: string[];
  /** Why a goal that did not complete stopped. */
  reason: string | null;
  /** A pause or clear that applies when the turn in flight ends. */
  pending: "pause" | "clear" | null;
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
  permission_mode?: string;
  effort?: string | null;
  tier: string | null;
  model: string;
  /** Additive: whether `model` is the user's pin rather than the mesh's latest pick. Absent from
   * older hosts — treat as unknown, not as unpinned. */
  model_pinned?: boolean;
  cost_usd: number;
  context_tokens: number;
  context_limit: number | null;
  /** The in-flight reply. The wire carries only its last 2 KiB; `useSessionSocket` rebuilds the
   * whole reply from `streaming_offset`, so consumers always see the full text so far. */
  streaming: string;
  /** Additive: UTF-16 units of the reply before the wire's `streaming` tail (0/absent: none). */
  streaming_offset?: number;
  /** Additive: what the running turn is doing ("model reasoning", "running tool"); "" idle. */
  activity?: string;
  /** Additive: when the running turn started, Unix ms; 0/absent when idle or unknown. */
  turn_started_at?: number;
  transcript: string[];
  /** v9 additive: `transcript` with per-line provenance. Absent/empty from a pre-v9 host —
   * fall back to `transcript` rather than rendering nothing. */
  transcript_rows?: TranscriptRow[];
  tasks: SnapshotTask[];
  subagents: SnapshotSubagent[];
  // v8.1 additive: active workflow-run state, null when no workflow is running.
  workflow?: SnapshotWorkflow | null;
  goal?: SnapshotGoal | null;
  queued: string[];
  permission_prompt: string | null;
  question: string | null;
  question_options: QuestionOption[];
  question_allow_other: boolean;
  /** v11 additive: the whole `ask_user` form. Absent from an older host — fall back to the
   * single `question` fields. */
  question_form?: FormQuestion[] | null;
  question_index?: number;
  overlay: Overlay | null;
  diff: Diff | null;
  plan: Plan | null;
  copy_text: string | null;
  /** AI-suggested likely next user prompt, refreshed after each completed turn. Absent/null
   * while none is available — never implies one is pending. */
  suggested_prompt?: string | null;
  prompt_seq: number;
  /** Rows that ever entered the host's live transcript tail; absent from an older host. */
  transcript_total?: number;
  notes: string[];
  revision: number;
  resync: boolean;
  closed: boolean;
  /** Set only on the snapshot of a subagent child session, naming the session it hangs off. */
  parent?: { id: string; title: string } | null;
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
  // "Queue, don't steer": waits behind the running turn and is never offered to it. Needs a
  // daemon with `chat.follow_up` (see lib/followUp.ts); an older one would drop the frame.
  | { kind: "queue"; text: string; attachments?: { path: string; image: boolean }[] }
  // Reorders / promotes one server-queued prompt; index+text echo Snapshot.queued like dequeue.
  | { kind: "queue_move"; index: number; text: string; to: number }
  | { kind: "queue_send_now"; index: number; text: string }
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
  u.protocol = u.protocol === "fany:" ? "fany-ws:" : u.protocol === "https:" ? "wss:" : "ws:";
  u.searchParams.set("session", sessionId);
  u.searchParams.set("rev", String(rev));
  return u.toString();
}

export interface UseSessionSocketResult {
  snapshot: Snapshot | null;
  connectionState: ConnectionState;
  send: (input: RemoteInput) => boolean;
  /** Reconnect a socket the daemon marked `closed`, from a fresh revision. */
  reopen: () => void;
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
  // One committed snapshot per paint (lib/frameCoalescer.ts); the streamed reply is rebuilt from
  // every frame first, so coalescing never loses text.
  const [frames] = useState(() => createFrameCoalescer<Snapshot>(setSnapshot));
  const streamingRef = useRef<StreamingAccumulator>(EMPTY_STREAMING);

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
    let ws: WebSocket;
    try {
      ws = new TWebSocket(wsUrl(baseUrl, sessionId, revRef.current));
    } catch {
      // A constructor that throws (a transport rejecting the URL, a native socket module not
      // ready straight after foregrounding) fires no `onclose`, so nothing below would ever
      // schedule another attempt: the screen sat on "reconnecting…" until the app was reopened.
      // Treat it exactly like a failed connection and take the normal backoff path.
      setConnectionState(attemptRef.current >= UNREACHABLE_AFTER_ATTEMPTS ? "unreachable" : "reconnecting");
      const delay = BACKOFF_MS[Math.min(attemptRef.current, BACKOFF_MS.length - 1)];
      attemptRef.current += 1;
      clearReconnectTimer();
      reconnectTimerRef.current = setTimeout(connect, delay);
      return;
    }
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
      streamingRef.current = accumulateStreaming(streamingRef.current, data.streaming ?? "", data.streaming_offset);
      const fullStreaming = streamingDisplayText(streamingRef.current);
      frames.push(fullStreaming === data.streaming ? data : { ...data, streaming: fullStreaming });
      if (data.closed) {
        frames.flush();
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
  }, [baseUrl, sessionId, teardown, clearReconnectTimer, clearLivenessTimer, frames]);

  // Remembers which (server, session) this hook last connected for. When the effect re-runs for
  // the SAME key it is a resume — a hidden <Activity> (a kept-alive session the user navigated
  // away from) being shown again: its effects were cleaned up, closing the socket, so nothing
  // streamed while it was hidden. Keep the last snapshot on screen and the revision so the daemon
  // replays only the gap, instead of blanking to a skeleton and refetching everything.
  const connectedKeyRef = useRef<string | null>(null);

  useEffect(() => {
    const key = `${baseUrl}|${sessionId}`;
    const resuming = connectedKeyRef.current === key;
    connectedKeyRef.current = key;
    closedRef.current = false;
    attemptRef.current = 0;
    shouldRunRef.current = true;
    frames.cancel();
    if (!resuming) {
      revRef.current = 0;
      streamingRef.current = EMPTY_STREAMING;
      setSnapshot(null);
      setConnectionState("idle");
    }
    connect();
    return () => {
      shouldRunRef.current = false;
      frames.cancel();
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
      // Whatever backoff the last background-time failures accrued is stale: the user is looking
      // at the screen now, so the first attempt is immediate and the "unreachable" escalation
      // starts from zero.
      attemptRef.current = 0;
      resyncPendingRef.current = false;
      connect();
    };

    // Web has no AppState lifecycle — branch on the DOM's visibilitychange directly.
    if (Platform.OS === "web") {
      // `appStateRef` gates the liveness watchdog, and on web the AppState listener below never
      // runs — so the ref would keep forever whatever `AppState.currentState` reported at mount.
      // react-native-web derives that from `document.visibilityState`, which is "hidden" for a tab
      // opened in the background (⌘-click, session restore): the watchdog would then be disabled
      // for the whole life of the page and a half-open socket would freeze the session silently.
      // Mirror the DOM's visibility into the ref instead — seeded now, updated on every change.
      const syncAppState = (hidden: boolean) => {
        appStateRef.current = hidden ? "background" : "active";
      };
      const wasHidden = { current: document.visibilityState === "hidden" };
      syncAppState(wasHidden.current);
      const onVisibilityChange = () => {
        const isHidden = document.visibilityState === "hidden";
        syncAppState(isHidden);
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

  const reopen = useCallback(() => {
    if (!closedRef.current || !shouldRunRef.current) return;
    closedRef.current = false;
    // A resumed session is a new driver whose revisions start over.
    revRef.current = 0;
    attemptRef.current = 0;
    setConnectionState("reconnecting");
    connect();
  }, [connect]);

  return { snapshot, connectionState, send, reopen };
}
