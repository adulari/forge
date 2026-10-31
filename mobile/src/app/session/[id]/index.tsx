// Chat (T3.2). ARCHITECTURE.md §4.1.4 (CRITICAL — timeline source-of-truth + dedupe), §4.2
// (offline prompt queue). FEATURES.md §1.2/§1.3 (Snapshot fields / RemoteInput -> UI).
// DESIGN_SYSTEM.md §6 (PromptComposer, MessageRow, Kindle streaming).
//
// §4.1.4 dedupe rule, implemented literally (not content-matched): the timeline's source of
// truth is `useHistory` rows. The snapshot contributes ONLY (a) the live `streaming` edge while
// `busy`, always rendered, and (b) `transcript` tail lines as instant warm-start filler — used
// until history has actual rows. The two sources remain mutually exclusive, so there is nothing
// to de-duplicate by content. Turn-completion history invalidation (busy true->false) is already wired by the
// T3.1 session shell's `useTurnCompleted(snapshot)` call in `_layout.tsx` — not repeated here.
import { DeadPinBanner } from "../../../components/session/DeadPinBanner";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useIsFocused, useNavigation } from "expo-router";
import { ChevronDown, ChevronUp, MessageSquare, SearchX, WifiOff } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";

import type { SentAttachment } from "../../../components/chat/attach";
import CardSlot from "../../../components/chat/CardSlot";
import { ComposerDocks } from "../../../components/chat/ComposerDocks";
import { Composer } from "../../../components/chat/Composer";
import { MessageActionsMenu } from "../../../components/chat/MessageActionsMenu";
import { MessageActionsSheet } from "../../../components/chat/MessageActionsSheet";
import { StreamingAnswer, TurnActivity } from "../../../components/chat/LiveTurn";
import { displayMessageText, MessageRow, SpeakerTag } from "../../../components/chat/MessageRow";
import { ToolCallRow } from "../../../components/chat/ToolCallRow";
import { ToolGroupRow } from "../../../components/chat/ToolGroupRow";
import { TurnFooter } from "../../../components/chat/TurnFooter";
import { useHistoryAutofill } from "../../../components/chat/useHistoryAutofill";
import { useMessageJump } from "../../../components/chat/useMessageJump";
import { useChatFind } from "../../../components/chat/useChatFind";
import { usePromptRail } from "../../../components/chat/usePromptRail";
import { useTopRow } from "../../../components/chat/useTopRow";
import { PromptRail } from "../../../components/chat/PromptRail";
import { StickyPromptBar } from "../../../components/chat/StickyPromptBar";
import { FindBar } from "../../../components/find/FindBar";
import { IntegrityDocks } from "../../../components/session/IntegrityDocks";
import { useWorkbench } from "../../../components/workbench/WorkbenchProvider";
import { findFocusProps } from "../../../lib/find/findController";
import { useHistoryView } from "../../../lib/historyView";
import { requestMessageJump } from "../../../lib/messageJump";
import { railVisible } from "../../../lib/promptRail";
import { stickyPromptFor, type PromptEntry } from "../../../lib/stickyPrompt";
import { useFinePointer } from "../../../lib/useFinePointer";
import { ReasoningDisclosure } from "../../../components/chat/ReasoningDisclosure";
import { buildTranscript, type ToolInvocation } from "../../../lib/toolRows";
import { foldToolGroups, GroupMemo, type FoldedEntry, type ToolGroupEntry } from "../../../lib/toolGroups";
import { withTurnFooters } from "../../../lib/turnFooter";
import { useAppearancePreferences } from "../../../lib/appearancePreferences";
import { fillerHistoryRows } from "../../../lib/transcriptFiller";
import { QueueStrip } from "../../../components/chat/QueueStrip";
import { SubagentStrip } from "../../../components/session/SubagentStrip";
import { BoundedList } from "../../../components/ds/BoundedList";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { useToast } from "../../../components/ds/ToastHost";
import { type HistoryRow } from "../../../lib/api";
import { answerAlreadyShown, historyHasRealContentSince, shouldRetryEmptyHistory } from "../../../lib/historyMerge";
import { shouldMountTranscript } from "../../../lib/transcriptMountGate";
import { reconcilePendingMessages, unqueuedPendingMessages } from "../../../lib/sessionReconciler";
import { OFFLINE_QUEUE_CAP, newClientMsgId, offlineQueueKey, parseOfflineQueue, queuedPromptInputs, type QueuedPrompt } from "../../../lib/offlineQueue";
import { collapsePrompts } from "../../../lib/promptRecall";
import { type FollowUpMode } from "../../../lib/followUp";
import { useFollowUpSetting } from "../../../lib/useFollowUpSetting";
import { haptics } from "../../../lib/haptics";
import { useHistory, useLiveHistoryRefresh, useSessionAgents, useSessions } from "../../../lib/queries";
import { rewindDisabledReason, sessionsSharingDirectory, sharedWorktreeWarning } from "../../../lib/forkRequest";
import { useConnectionAccess } from "../../../lib/queriesAccess";
import { parseReasoning } from "../../../lib/reasoning";
import { AgentIndexProvider } from "../../../lib/agentLinks";
import { usePaneNav } from "../../../components/shell/paneNav";
import { useSessionCtx } from "../../../lib/sessionContext";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";
import { useBreakpoint } from "../../../theme/useBreakpoint";
import { useSegmentsHosted } from "../../../components/session/SegmentKeepAlive";

// T3.3's CardSlot.tsx landed during this task (was a HANDOFF stub) — wired in directly above.

const JUMP_THRESHOLD_PX = 240;
// How often an empty history page is re-asked for while the live snapshot shows a transcript.
const HISTORY_RECOVERY_POLL_MS = 5_000;
// Safety net for the Android transcript-mount gate below, in case `transitionEnd` never fires
// for this route (e.g. a screen that was already the active route with nothing to animate).
const TRANSCRIPT_MOUNT_FALLBACK_MS = 1_200;

type TimelineItem =
  | { kind: "streaming"; id: string; text: string; streaming: boolean }
  | { kind: "history"; id: string; row: HistoryRow }
  | { kind: "tool"; id: string; invocation: ToolInvocation }
  | { kind: "toolGroup"; id: string; group: ToolGroupEntry }
  | { kind: "turnFooter"; id: string; text: string }
  | { kind: "filler"; id: string; text: string }
  | { kind: "note"; id: string; text: string }
  | { kind: "pendingSent"; id: string; text: string; attachments: SentAttachment[] };

const EMPTY_TIMELINE: TimelineItem[] = [];
const NO_AGENTS: never[] = [];

// How long an optimistic "pendingSent" bubble is allowed to linger without a real history row
// ever landing for it (session closed mid-turn, WS never came back, etc.) — a safety net, not
// the normal clearing path (that's the historyRows-advanced-past-baseline effect below).
const PENDING_SENT_TIMEOUT_MS = 120_000;

interface PendingSent {
  id: string;
  text: string;
  /** `historyRows[0]?.seq` at send time — cleared once a newer row lands. */
  baselineSeq: number;
  /** Attachments that rode this prompt (already uploaded — Composer only ever hands over
   * `state === "done"` ones), rendered on the bubble since the daemon never persists them. */
  attachments: SentAttachment[];
}

export function SessionChat() {
  const tokens = useTokens();
  const paneNav = usePaneNav();
  const toast = useToast();
  const { sessionId, baseUrl, snapshot, snapshotTimedOut, connectionState, send, headerHeight, pendingAnswer, clearPendingAnswer, getDraftText, setDraftText, focusComposer } = useSessionCtx();

  // The chat opts into tool rows so a turn's tool activity renders inline (ToolCallRow),
  // instead of the tool-less stream the fork picker and turn watcher still use.
  const historyQuery = useHistory(sessionId, { includeTools: true });
  // The newest rows the snapshot carries change when a step finishes — the moment the history
  // page has something new. Streaming text is deliberately not part of it.
  const liveRowsSignal = useMemo(
    () =>
      (snapshot?.transcript_rows ?? [])
        .slice(-2)
        .map((row) => `${row.kind}:${row.tool ?? ""}:${row.text.length}`)
        .join("|") + `#${snapshot?.transcript.length ?? 0}#${snapshot?.transcript_total ?? 0}`,
    [snapshot?.transcript_rows, snapshot?.transcript.length, snapshot?.transcript_total],
  );
  useLiveHistoryRefresh(sessionId, snapshot?.busy ?? false, liveRowsSignal, true);
  // `read_only` means there is genuinely no input path: a terminal session whose Forge build
  // predates the control channel, or whose user opted out of it. A terminal session that DID
  // publish one reports `read_only: false` — the daemon proxies this WS straight into it — so the
  // composer mounts and drives it exactly like a daemon-hosted session. The WS route 403s the
  // input-less case server-side too (`reject_ws_if_local_read_only` in `serve.rs`). The fleet list
  // is the only place that carries the flag, so look this session up there rather than adding it
  // to every other response shape.
  const agentsSignal = (snapshot?.subagents ?? []).map((s) => (s.done ? "d" : "r")).join("");
  const agentIndex = useSessionAgents(snapshot?.parent ? null : sessionId, agentsSignal).data ?? NO_AGENTS;
  const sessionsQuery = useSessions();
  const parentSession = snapshot?.parent ?? null;
  const connectionAccess = useConnectionAccess();
  const readOnly =
    parentSession != null ||
    connectionAccess.viewOnly ||
    (sessionsQuery.data?.find((row) => row.id === sessionId)?.read_only ?? false);
  const [selectedMessage, setSelectedMessage] = useState<HistoryRow | null>(null);
  // Pointer-driven opens (right-click / hover ⋯) carry an anchor → compact popover menu;
  // long-press carries none → bottom sheet. Web-only distinction, native never sends one.
  const [actionsAnchor, setActionsAnchor] = useState<{ x: number; y: number } | null>(null);

  const onMessageLongPress = useCallback((message: HistoryRow, anchor?: { x: number; y: number }) => {
    setSelectedMessage(message);
    setActionsAnchor(anchor ?? null);
  }, []);

  const onQuote = useCallback((text: string) => {
    const quote = text.split("\n").map((line) => `> ${line}`).join("\n");
    const draftText = getDraftText();
    setDraftText(draftText ? `${draftText}\n${quote}` : quote);
    focusComposer();
  }, [focusComposer, getDraftText, setDraftText]);

  // Edit & resend: the message text REPLACES the draft (unlike quote, which appends).
  const onEditMessage = useCallback((text: string) => {
    setDraftText(text);
    focusComposer();
  }, [focusComposer, setDraftText]);

  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [showJump, setShowJump] = useState(false);
  const [newWhileAway, setNewWhileAway] = useState(0);
  const showJumpRef = useRef(false);
  useEffect(() => {
    showJumpRef.current = showJump;
  }, [showJump]);

  // ---------------------------------------------------------------------
  // Android modal-dismiss reparenting race (see transcriptMountGate.ts doc). `new-session.tsx`
  // reaches this screen via `dismissTo`, which dismisses its modal and pushes this screen in one
  // native stack transition; the daemon-run first turn means real history rows can now land and
  // want to mount into the transcript FlatList while that transition is still moving views
  // around, which is what throws Fabric's "already has a parent" and kills the React instance.
  //
  // `transitionEnd` (react-native-screens' native onAppear signal for THIS route, closing:false)
  // is the actual "the native transition settled" event — unlike `InteractionManager`, which
  // native-stack's real (non-JS-animated) transitions never register a handle with, so it would
  // resolve immediately and not gate anything. `useNavigation()`'s default typing doesn't carry
  // native-stack's event map without reaching into expo-router's internal module path, hence the
  // narrow local cast below instead of an internal deep import.
  // ---------------------------------------------------------------------
  const isFocused = useIsFocused();
  const navigation = useNavigation();
  const [transitionSettled, setTransitionSettled] = useState(() => Platform.OS !== "android");
  useEffect(() => {
    if (transitionSettled) return;
    const settle = () => setTransitionSettled(true);
    const withTransitionEvents = navigation as unknown as {
      addListener: (type: "transitionEnd", listener: (event: { data: { closing: boolean } }) => void) => () => void;
    };
    const unsubscribe = withTransitionEvents.addListener("transitionEnd", (event) => {
      if (!event.data.closing) settle();
    });
    const fallback = setTimeout(settle, TRANSCRIPT_MOUNT_FALLBACK_MS);
    return () => {
      unsubscribe();
      clearTimeout(fallback);
    };
  }, [transitionSettled, navigation]);
  const transcriptReady = shouldMountTranscript({
    isAndroid: Platform.OS === "android",
    isFocused,
    transitionSettled,
  });

  // ---------------------------------------------------------------------
  // Offline prompt queue (ARCHITECTURE §4.2): per server+session AsyncStorage queue, cap 20,
  // flushed in order on the exact reconnect edge, rendered as "queued (offline)" chips above
  // the composer. Loud drop past the cap (toast + haptic, never a silent no-op).
  // ---------------------------------------------------------------------
  const [offlineQueue, setOfflineQueue] = useState<QueuedPrompt[]>([]);
  const offlineLoadedRef = useRef(false);

  useEffect(() => {
    offlineLoadedRef.current = false;
    let cancelled = false;
    AsyncStorage.getItem(offlineQueueKey(baseUrl, sessionId)).then((raw) => {
      if (cancelled) return;
      setOfflineQueue(parseOfflineQueue(raw));
      offlineLoadedRef.current = true;
    });
    return () => {
      cancelled = true;
    };
  }, [baseUrl, sessionId]);

  useEffect(() => {
    if (!offlineLoadedRef.current) return;
    AsyncStorage.setItem(offlineQueueKey(baseUrl, sessionId), JSON.stringify(offlineQueue)).catch(() => {
      // best-effort persistence; in-memory queue is still authoritative for this session
    });
  }, [offlineQueue, baseUrl, sessionId]);

  // One-shot interrupt latched while the socket was closed — flushed on the exact !open→open
  // edge (below) so it reaches the daemon the moment the connection is restored. Cleared by the
  // flush effect (not by the interrupt itself, so a second interrupt before reconnect replaces
  // the first rather than duplicating it).
  const [pendingInterrupt, setPendingInterrupt] = useState(false);

  // Flush queued prompts in order whenever the socket is open and a queue exists — on the reconnect
  // edge AND when a queue was restored from storage while ALREADY open (e.g. remounting Chat after a
  // segment switch during which the persistent socket reconnected). Gating purely on the `!open→open`
  // transition left such a restored queue stuck as a permanent "(offline)" chip while actually online.
  const prevConnRef = useRef(connectionState);
  useEffect(() => {
    const was = prevConnRef.current;
    prevConnRef.current = connectionState;
    if (connectionState !== "open" || !offlineLoadedRef.current) return;
    if (pendingAnswer) {
      if (snapshot?.prompt_seq === pendingAnswer.seq && (snapshot.permission_prompt != null || snapshot.question != null)) {
        if (send(pendingAnswer)) clearPendingAnswer();
      } else {
        clearPendingAnswer();
      }
    }
    if (offlineQueue.length > 0) {
      for (const input of queuedPromptInputs(offlineQueue)) send(input);
      setOfflineQueue([]);
    }
    // The pending interrupt is edge-only: it means "Stop pressed while offline", so it fires on the
    // reconnect edge, not on a plain remount-while-online.
    if (pendingInterrupt && was !== "open") {
      send({ kind: "interrupt" });
      setPendingInterrupt(false);
    }
  }, [connectionState, offlineQueue, pendingInterrupt, pendingAnswer, snapshot?.prompt_seq, snapshot?.permission_prompt, snapshot?.question, send, clearPendingAnswer]);

  const online = connectionState === "open";

  // Optimistic "sent" bubble (ARCHITECTURE §4.1.4 timeline is otherwise server-truth-only):
  // without this, the user's own message doesn't appear anywhere until the whole turn
  // completes and history is invalidated/refetched — the composer looked like it swallowed
  // the prompt. `latestSeqRef` mirrors `historyRows[0]?.seq` (computed further down) so
  // `handleSend`, defined here, can read its current value without reordering the file.
  const [pendingSent, setPendingSent] = useState<PendingSent[]>([]);
  const latestSeqRef = useRef(-1);
  const returnToLatestRef = useRef<() => void>(() => {});

  const handleSend = useCallback(
    (text: string, attachments: SentAttachment[], mode: FollowUpMode = "steer"): boolean => {
      const id = `p${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;

      // Only uploaded (path-bearing) attachments can ride the wire — correlate this exact send's
      // attachments to this exact prompt (ARCHITECTURE §4.1.4 / mobile upload race).
      const withPath = attachments
        .filter((a): a is SentAttachment & { path: string } => a.path != null)
        .map((a) => ({ path: a.path, image: a.image }));
      // A side question's answer is a card and a `/goal` is a banner, not transcript rows, so
      // their text never gets a bubble that no history row would ever claim.
      const sideQuestion = /^\/(btw|side|goal)(\s|$)/i.test(text);
      const clientMsgId = newClientMsgId();
      if (send({ kind: mode === "queue" ? "queue" : "prompt", text, attachments: withPath, client_msg_id: clientMsgId })) {
        returnToLatestRef.current();
        if (!sideQuestion) setPendingSent((prev) => [...prev, { id, text, baselineSeq: latestSeqRef.current, attachments }]);
        return true;
      }
      // Socket closed between the `online` render and here (race window). Fall through to
      // the offline queue — it will be flushed in order on the next !open→open edge.
      if (offlineQueue.length >= OFFLINE_QUEUE_CAP) {
        toast.show("offline queue full (20) — prompt dropped", { tone: "danger" });
        haptics.mergeConflict();
        return false;
      }
      setPendingSent((prev) => [...prev, { id, text, baselineSeq: latestSeqRef.current, attachments }]);
      setOfflineQueue((prev) => [...prev, { text, attachments: withPath, clientMsgId }]);
      return true;
    },
    [offlineQueue.length, send, toast],
  );

  const handleInterrupt = useCallback(() => {
    // Try to send immediately; if the socket is closed, latch a pending interrupt that the
    // !open→open flush effect will deliver the moment the connection is restored.
    if (!send({ kind: "interrupt" })) setPendingInterrupt(true);
  }, [send]);

  // ---------------------------------------------------------------------
  // Timeline construction (ARCHITECTURE §4.1.4)
  // ---------------------------------------------------------------------
  const historyRows = useMemo<HistoryRow[]>(() => {
    const pages = historyQuery.data?.pages ?? [];
    // Render ALL rows: `visibility` is "llm" for normal turns and "ui" for user-facing
    // notes — BOTH are part of the visible conversation (remote.rs HistoryRow doc; the web
    // PWA renders every row, styling "ui" as a note). Filtering to one drops the conversation.
    return pages.flat();
  }, [historyQuery.data]);

  const view = useHistoryView(baseUrl, sessionId, historyQuery, historyRows);
  const shownRows = view.rows;
  useEffect(() => {
    returnToLatestRef.current = view.returnToLatest;
  });
  const [findSeq, setFindSeq] = useState<number | null>(null);
  const workbench = useWorkbench();
  const findEnabled = useIsFocused() && (workbench.focusedSessionId == null || workbench.focusedSessionId === sessionId);
  useChatFind({ enabled: findEnabled, baseUrl, sessionId, rows: shownRows, onActivate: setFindSeq });

  const followUp = useFollowUpSetting();
  const recallPrompts = useMemo(
    () =>
      collapsePrompts(
        historyRows
          .filter((row) => row.role === "user" && row.kind !== "nudge" && row.visibility === "llm")
          .map(displayMessageText)
          .reverse(),
      ),
    [historyRows],
  );

  useEffect(() => {
    latestSeqRef.current = historyRows[0]?.seq ?? -1;
  }, [historyRows]);

  // Clear a pendingSent bubble only once ITS OWN user row has landed in history — a `role:"user"`
  // row newer than the send's baseline whose content matches the bubble's text (ARCHITECTURE
  // §4.1.4 invalidation). Clearing on ANY history advance (an unrelated note row, a concurrent
  // turn) would drop the optimistic bubble before its real row exists, flashing the message out.
  useEffect(() => {
    setPendingSent((prev) => reconcilePendingMessages(prev, historyRows));
  }, [historyRows]);

  // Safety net: never let a pendingSent bubble linger forever if history never advances for it
  // (session closed mid-turn, connection never came back, ...). A normal successful send clears
  // its timer here too (the effect above already removed it from `pendingSent`, which reruns
  // this effect and cancels the stale timer before it can fire) — so the toast only ever fires
  // for the genuine stuck case, not on every ordinary turn.
  const queuedKey = (snapshot?.queued ?? []).join("\u0000");
  const shownPending = useMemo(() => unqueuedPendingMessages(pendingSent, queuedKey ? queuedKey.split("\u0000") : []), [pendingSent, queuedKey]);
  useEffect(() => {
    if (shownPending.length === 0) return;
    const timers = shownPending.map((p) =>
      setTimeout(() => {
        setPendingSent((prev) => prev.filter((x) => x.id !== p.id));
        toast.show("message didn't confirm — check connection", { tone: "danger" });
      }, PENDING_SENT_TIMEOUT_MS),
    );
    return () => timers.forEach(clearTimeout);
  }, [shownPending, toast]);

  // REST history is authoritative once populated. A successful-but-empty page is not evidence
  // that the daemon transcript is empty, so retain the snapshot tail until history has rows.
  const historySettled = historyQuery.data !== undefined;
  const useTranscriptFiller = !historySettled || historyRows.length === 0;
  const historyEmpty = historySettled && historyRows.length === 0 && (snapshot?.transcript.length ?? 0) === 0;

  // A deep link into a nonexistent/foreign session: `useHistory` 404s and no Snapshot ever
  // arrives over WS. All three conditions matter — `!historySettled` and `snapshot == null`
  // keep a transient history error on an otherwise-live session (cached history or a WS
  // snapshot already present) from flashing this scary state.
  const historyDead = historyQuery.isError && !historySettled && snapshot == null;
  const historyErrorStatus = (historyQuery.error as { status?: number } | null)?.status;

  // History recovery. Two ways the screen used to get stuck on the transcript filler until a
  // turn happened to complete (the only invalidation): (1) the socket reconnected after an
  // outage/daemon restart but the failed history query was never retried — react-query's
  // reconnect refetch keys off a browser online event RN never fires; (2) history answered with
  // an EMPTY page (a session the restarted daemon had not re-registered yet) while the live
  // snapshot plainly had content. Refetch on the reconnect edge, and poll a contradicted-empty
  // history until it agrees with the snapshot.
  const historyRefetch = historyQuery.refetch;
  const prevOnlineRef = useRef(online);
  useEffect(() => {
    const was = prevOnlineRef.current;
    prevOnlineRef.current = online;
    if (online && !was) void historyRefetch();
  }, [online, historyRefetch]);
  // Blind-attempt counter for `shouldRetryEmptyHistory` — reset whenever this screen shows real
  // rows, or switches to a different session. The session-id reset is state adjusted during
  // render (same pattern `track` below uses — https://react.dev/learn/you-might-not-need-an-effect
  // — not a ref, since reading/writing a ref during render is unsound), so the very first render
  // for a new session never reads a stale count left over from the last one.
  const [historyRecovery, setHistoryRecovery] = useState({ sessionId, attempts: 0 });
  if (historyRecovery.sessionId !== sessionId) {
    setHistoryRecovery({ sessionId, attempts: 0 });
  }
  useEffect(() => {
    if (historyRows.length === 0) return;
    setHistoryRecovery((prev) => (prev.attempts === 0 ? prev : { ...prev, attempts: 0 }));
  }, [historyRows.length]);

  const historyContradicted = shouldRetryEmptyHistory({
    historySettled: historyQuery.data !== undefined,
    rowsEmpty: historyRows.length === 0,
    snapshotHasContent: (snapshot?.transcript.length ?? 0) > 0,
    blindAttempts: historyRecovery.attempts,
  });
  useEffect(() => {
    if (!historyContradicted) return;
    // Fire once right away — a freshly created session's chat screen otherwise waits a full
    // `HISTORY_RECOVERY_POLL_MS` before its first correction attempt, which read as "renders
    // nothing until you leave and come back" when the daemon's rows landed just after the
    // screen's own first (empty) fetch.
    void historyRefetch();
    const timer = setInterval(() => {
      setHistoryRecovery((prev) => ({ ...prev, attempts: prev.attempts + 1 }));
      void historyRefetch();
    }, HISTORY_RECOVERY_POLL_MS);
    return () => clearInterval(timer);
  }, [historyContradicted, historyRefetch]);

  const streamingText = snapshot?.busy ? snapshot.streaming : "";

  // ---------------------------------------------------------------------
  // Bridge the busy(true)->false gap (see module doc above): the instant `busy` flips false,
  // `streamingText` above goes to "" but the finalized turn hasn't landed in `historyRows` yet
  // (that only happens once `useTurnCompleted`'s invalidation refetch resolves). Without help,
  // the just-finished message would vanish from both sources for one or more frames, then pop
  // back in from history — the reported flicker.
  //
  // The retain-on-busy-edge step below runs during render (React's documented "adjust state
  // while rendering" pattern — https://react.dev/learn/you-might-not-need-an-effect), NOT in a
  // `useEffect`: an effect only runs *after* React has already committed (and painted) the
  // render where `busy` just flipped false, so the gap frame would still exist for one paint.
  // Calling `setTrack` synchronously here instead makes React throw away that in-between
  // render and immediately re-render with the retained text already in place — nothing is ever
  // committed with the message absent. Dropping the retained text once the finalized row lands
  // is not paint-critical (the render-time `finalizingActive` check further below already stops
  // rendering it the instant `historyRows` advances), so that half uses ordinary effects.
  // ---------------------------------------------------------------------
  const busy = snapshot?.busy ?? false;
  const rewindBlocked = rewindDisabledReason(busy);
  const rewindWarning = useMemo(
    () => sharedWorktreeWarning(sessionsSharingDirectory(sessionsQuery.data, sessionId)),
    [sessionsQuery.data, sessionId],
  );
  // `/rewind` opens the server's checkpoint picker; selecting the message's own row jumps to the
  // restore-scope step, so the person only chooses chat / files / both. The prompt comes back
  // into the composer the way the terminal puts it back in its input.
  const onRewind = useCallback((message: HistoryRow) => {
    if (!send({ kind: "prompt", text: "/rewind" })) {
      toast.show("Not connected — could not start the rewind", { tone: "danger" });
      return;
    }
    send({ kind: "overlay_select", id: String(message.seq) });
    setDraftText(displayMessageText(message));
    focusComposer();
  }, [focusComposer, send, setDraftText, toast]);

  const [track, setTrack] = useState(() => ({
    sessionId,
    busy,
    retainedText: streamingText,
    finalizing: null as { text: string; baselineSeq: number } | null,
  }));

  if (track.sessionId !== sessionId) {
    // Session switch: nothing carries over.
    setTrack({ sessionId, busy, retainedText: streamingText, finalizing: null });
  } else if (busy !== track.busy) {
    setTrack(
      busy
        ? { sessionId, busy, retainedText: streamingText, finalizing: null }
        : {
            sessionId,
            busy,
            retainedText: track.retainedText,
            finalizing: track.retainedText
              ? { text: track.retainedText, baselineSeq: historyRows[0]?.seq ?? -1 }
              : null,
          },
    );
  } else if (busy && streamingText && streamingText !== track.retainedText) {
    // `streamingText` guard: the real daemon can (and does — verified live) send a snapshot
    // with `streaming` already cleared to "" one tick BEFORE the `busy` true->false edge
    // itself, not atomically with it. Without this guard that empty tick would blow away
    // `retainedText` right before the edge, so `finalizing` below (`track.retainedText ? … :
    // null`) would never arm and the just-finished reply would render as nothing at all —
    // not a brief flicker but a real gap until the history refetch resolves. Only ever
    // replace the retained text with another real chunk; a transient empty one is ignored.
    setTrack({ ...track, retainedText: streamingText });
  }

  const { finalizing } = track;

  // Clear once the finalized row has actually arrived with REAL content (state cleanup only —
  // see comment above). Checking for any seq advancement at all (rather than real content)
  // cleared this the instant a blank placeholder row landed — a known daemon gap where a turn's
  // prose lives only on tool rows this page didn't fetch — dropping the reply from view entirely
  // (it's not shown here via the bridge anymore, and buildTranscript filters the blank row out of
  // the rendered history too). historyHasRealContentSince keeps the bridge up until a row with
  // actual content shows up; the timeout below still bounds how long that can take.
  useEffect(() => {
    if (!finalizing) return;
    if (historyHasRealContentSince(historyRows, finalizing.baselineSeq)) {
      setTrack((prev) => (prev.finalizing === finalizing ? { ...prev, finalizing: null } : prev));
    }
  }, [finalizing, historyRows]);

  // Never-get-stuck safety net: if history doesn't settle within a few seconds, drop it anyway
  // rather than let it linger as a stale duplicate of whatever eventually lands.
  useEffect(() => {
    if (!finalizing) return;
    const t = setTimeout(() => {
      setTrack((prev) => (prev.finalizing === finalizing ? { ...prev, finalizing: null } : prev));
    }, 4000);
    return () => clearTimeout(t);
  }, [finalizing]);

  // Stays active until REAL content lands (not merely until `historyRows[0]` advances at all —
  // see historyHasRealContentSince): a blank placeholder row landing first must not blank out
  // this bridge before the real reply is actually visible anywhere.
  const finalizingActive =
    !busy && finalizing !== null && !historyHasRealContentSince(historyRows, finalizing.baselineSeq);
  // Bridge the mid-busy empty tick too (root cause of the residual flicker): the daemon flushes
  // the reply out of `streaming` into `transcript` one or more frames BEFORE `busy` flips false
  // (verified live — `streaming` goes "" while still busy). Falling back to `track.retainedText`
  // while busy keeps the just-streamed answer on screen through those frames instead of blanking
  // it back to the "thinking…" indicator, then `finalizing` carries it across the busy->false
  // edge until the history row lands. No frame ever renders the answer absent (`displayText`, below the folds).

  // The newest live tool line (`↳ shell {...}`), by value, so a fresh transcript array per frame
  // does not change `renderItem`'s identity and re-render every visible cell.
  const transcriptTail = snapshot?.transcript;
  const liveToolLine = useMemo(() => {
    const lines = transcriptTail ?? [];
    for (let i = lines.length - 1; i >= 0; i--) {
      const line = lines[i].trim();
      if (line.startsWith("↳ ")) return line;
    }
    return null;
  }, [transcriptTail]);
  // An older host sends no `activity`; a tool line is then the only sign of what the turn does.
  const activity = snapshot?.activity || undefined;
  const turnStartedAt = snapshot?.turn_started_at ?? 0;
  const activityToolLine = !activity || activity === "running tool" ? liveToolLine : null;

  const transcriptRows = snapshot?.transcript_rows;
  // `historyRows` come from react-query and are referentially stable between refetches, so this
  // only recomputes when history actually changes — not on every ~30ms WS snapshot that
  // re-renders SessionChat while streaming.
  const historyTranscript = useMemo(() => buildTranscript(shownRows), [shownRows]);
  const inFlight = busy || streamingText.length > 0 || pendingSent.length > 0;
  const fillerTranscript = useMemo(() => {
    if (!transcriptRows?.length) return [];
    const rows = fillerHistoryRows(transcriptRows, inFlight) ?? [];
    return buildTranscript(rows);
  }, [transcriptRows, inFlight]);
  // Runs of tool calls fold into one summary row (lib/toolGroups.ts). Folded here, off the
  // transcripts, not inside `items`: `items` rebuilds on every streamed token, the transcripts
  // only when history changes, and each memo keeps a group's object while its calls are
  // unchanged, so a refetch re-summarises only the group that moved.
  const { foldToolGroups: foldEnabled } = useAppearancePreferences();
  const [historyGroupMemo] = useState(() => new GroupMemo());
  const [fillerGroupMemo] = useState(() => new GroupMemo());
  const foldedHistory = useMemo<FoldedEntry[]>(
    () => (foldEnabled ? foldToolGroups(historyTranscript, busy, historyGroupMemo) : historyTranscript),
    [foldEnabled, historyTranscript, busy, historyGroupMemo],
  );
  const foldedFiller = useMemo<FoldedEntry[]>(
    () => (foldEnabled ? foldToolGroups(fillerTranscript, busy, fillerGroupMemo) : fillerTranscript),
    [foldEnabled, fillerTranscript, busy, fillerGroupMemo],
  );
  const timelineHistory = useMemo(() => withTurnFooters(foldedHistory, busy), [foldedHistory, busy]);
  const retained = busy && !answerAlreadyShown(useTranscriptFiller ? foldedFiller : foldedHistory, track.retainedText);
  const displayText = streamingText || (retained ? track.retainedText : "") || (finalizingActive ? finalizing!.text : "");
  // Open groups by id (the id of the group's first call), so they stay open while the list
  // re-renders, virtualises cells away and grows.
  const [openGroups, setOpenGroups] = useState<ReadonlyMap<string, boolean>>(() => new Map());
  const toggleGroup = useCallback((id: string, next: boolean) => {
    setOpenGroups((prev) => new Map(prev).set(id, next));
  }, []);
  const items = useMemo<TimelineItem[]>(() => {
    const list: TimelineItem[] = [];
    // `busy` alone (before any tokens arrive) still gets a "streaming" slot — rendered with
    // empty text as the thinking indicator below — so there's never a silent gap between
    // submit and the first token (previously nothing rendered here at all until `displayText`
    // was non-empty, which read as "stuck").
    const atLive = !view.detached;
    if (atLive && (displayText || busy)) {
      list.push({ kind: "streaming", id: "streaming", text: displayText, streaming: busy || Boolean(streamingText) });
    }
    // Notes (turn-end notices, e.g. "compacting", "turn failed") are the NEWEST events — they must
    // sit at the newest slot (just under the streaming reply / above the composer in the inverted
    // list), not be pushed to the far/oldest end where they'd float above all history with no cue.
    const historyNoteTexts = new Set(
      historyRows.filter((row) => row.visibility === "ui").map((row) => row.content),
    );
    const notes = atLive ? (snapshot?.notes ?? []).filter((note) => !historyNoteTexts.has(note)) : [];
    for (let i = notes.length - 1; i >= 0; i--) {
      list.push({ kind: "note", id: `n${i}`, text: notes[i] });
    }
    // Newest-first (inverted list): the user's own just-sent message is more recent than any
    // settled history row but older than the in-progress reply above, and later sends are newer
    // than earlier ones — walk pendingSent back-to-front.
    for (const p of atLive ? [...shownPending].reverse() : []) list.push({ kind: "pendingSent", id: p.id, text: p.text, attachments: p.attachments });
    if (!useTranscriptFiller) {
      // Collapse the call/result row PAIR the daemon serves into one ToolCallRow entry, and keep
      // ordinary turns as message rows. `historyRows` is already newest-first, which is what
      // buildTranscript takes and returns.
      for (const entry of timelineHistory) {
        if (entry.kind === "turnFooter") {
          list.push({ kind: "turnFooter", id: entry.key, text: entry.text });
        } else if (entry.kind === "toolGroup") {
          list.push({ kind: "toolGroup", id: entry.key, group: entry });
        } else if (entry.kind === "tool") {
          list.push({ kind: "tool", id: entry.key, invocation: entry.invocation });
        } else {
          list.push({ kind: "history", id: `h${entry.row.seq}`, row: entry.row });
        }
      }
    } else if (transcriptRows?.length) {
      // A v9 host sends the transcript with provenance: render it through the SAME message/tool
      // rows history uses, so a late/failed history fetch is invisible instead of flipping the
      // whole conversation to plain grey lines (the reported "suddenly all grey" state). Seqs are
      // synthetic; ids are namespaced so they never collide with real history items.
      for (const entry of foldedFiller) {
        if (entry.kind === "toolGroup") {
          list.push({ kind: "toolGroup", id: `f${entry.key}`, group: entry });
        } else if (entry.kind === "tool") {
          list.push({ kind: "tool", id: `f${entry.key}`, invocation: entry.invocation });
        } else {
          list.push({ kind: "history", id: `fh${entry.row.seq}`, row: entry.row });
        }
      }
    } else {
      // Pre-v9 host: only plain lines are available. Snapshot transcript is chronological
      // (oldest->newest); walk it back-to-front for the inverted list. It remains visible after
      // an empty history response because that response cannot prove the server transcript is
      // empty.
      let filler = snapshot?.transcript ?? [];
      // The tail's newest lines describe the CURRENT turn, which the styled rows above
      // (pendingSent bubble + streaming reply + TurnActivity) already render — keeping
      // them here painted the same exchange twice, once as plain lines. Cut the filler at
      // the current turn's start (the last "you" marker) while a turn is in flight.
      if (busy || streamingText || shownPending.length > 0) {
        const lastYou = filler.map((l) => l.trim()).lastIndexOf("you");
        if (lastYou >= 0) filler = filler.slice(0, lastYou);
      }
      for (let i = filler.length - 1; i >= 0; i--) {
        list.push({ kind: "filler", id: `f${i}`, text: filler[i] });
      }
    }
    return list;
  }, [view.detached, displayText, streamingText, busy, shownPending, useTranscriptFiller, historyRows, timelineHistory, foldedFiller, snapshot?.transcript, transcriptRows, snapshot?.notes]);

  // Pin to the latest item when a NEW item lands at the newest slot — not on every streaming
  // text tick (same item id, StreamingAnswer owns its own rAF coalescing), and not when an older
  // page is appended at the far end (that only changes the last item, not the first).
  const newestKeyRef = useRef<string | null>(null);
  const detachedRef = useRef(view.detached);
  useEffect(() => {
    const newestKey = items[0]?.id ?? null;
    const swapped = detachedRef.current !== view.detached;
    detachedRef.current = view.detached;
    if (newestKey !== newestKeyRef.current) {
      newestKeyRef.current = newestKey;
      if (swapped) return;
      if (!showJumpRef.current) {
        listRef.current?.scrollToOffset({ offset: 0, animated: false });
      } else {
        setNewWhileAway((count) => count + 1);
      }
    }
  }, [items, view.detached]);

  const topRow = useTopRow(items);
  const { onRowLayout, onScroll: onTopRowScroll, onViewport } = topRow;
  const autofill = useHistoryAutofill(
    view,
    transcriptReady && !useTranscriptFiller,
    historyRows.length,
  );
  const { onScrollNearEnd } = autofill;
  const onScrollToIndexFailed = useMessageJump(sessionId, items, listRef, view, view.jumpAround);
  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    onScrollNearEnd(e.nativeEvent);
    onTopRowScroll(e.nativeEvent.contentOffset.y, e.nativeEvent.layoutMeasurement.height);
    const awayFromLatest = e.nativeEvent.contentOffset.y > JUMP_THRESHOLD_PX;
    setShowJump(awayFromLatest);
    if (!awayFromLatest) {
      setShowResponseNav(false);
      return;
    }
    setShowResponseNav(true);
  }, [onScrollNearEnd, onTopRowScroll]);

  const returnToLatest = view.returnToLatest;
  const jumpToLatest = useCallback(() => {
    returnToLatest();
    setShowJump(false);
    setNewWhileAway(0);
    listRef.current?.scrollToOffset({ offset: 0, animated: true });
  }, [returnToLatest]);

  const renderRow = useCallback(
    (item: TimelineItem) => {
      switch (item.kind) {
        case "history":
          return item.row.seq === findSeq ? (
            <View style={{ backgroundColor: tokens.selection }}>
              <MessageRow row={item.row} onLongPress={onMessageLongPress} />
            </View>
          ) : (
            <MessageRow row={item.row} onLongPress={onMessageLongPress} />
          );
        case "tool":
          return <ToolCallRow invocation={item.invocation} />;
        case "turnFooter":
          return <TurnFooter text={item.text} />;
        case "toolGroup":
          return <ToolGroupRow group={item.group} expanded={openGroups.get(item.group.key)} onToggle={toggleGroup} />;
        case "pendingSent":
          // Renders through the same MessageRow the real (server-truth) row will use once
          // history lands, so there's no visual "jump" when this optimistic bubble is replaced.
          // `attachments` rides separately (not embedded in `row.content`) since the daemon
          // never persists them — see MessageRowProps' doc comment.
          return (
            <MessageRow
              row={{
                seq: -1,
                role: "user",
                content: item.text,
                model: null,
                created_at: Date.now() / 1000,
                visibility: "llm",
              }}
              attachments={item.attachments}
              onLongPress={onMessageLongPress}
            />
          );
        case "streaming": {
          // Split inline `<think>…</think>` out of the live text: reasoning goes to the collapsed
          // disclosure, only the answer streams in the main slot (never the raw thinking log).
          const parsed = parseReasoning(item.text);
          const hasReasoning = parsed.reasoning.length > 0 || parsed.thinking;
          // What the turn is doing, while no answer text is arriving: before the first token,
          // and between steps (a tool running after some prose). Never a bare spinner.
          const showActivity = item.streaming && busy && (!parsed.answer || (activity != null && activity !== "streaming response"));
          if (!hasReasoning && !parsed.answer && !showActivity) return null;
          return (
            <View style={styles.streamingRow}>
              {parsed.answer ? <SpeakerTag speaker="forge" /> : null}
              {hasReasoning ? (
                <ReasoningDisclosure
                  reasoning={parsed.reasoning}
                  active={parsed.thinking && item.streaming}
                />
              ) : null}
              {parsed.answer ? (
                <StreamingAnswer text={parsed.answer} streaming={item.streaming && !showActivity} />
              ) : null}
              {showActivity ? <TurnActivity activity={activity} toolLine={activityToolLine} startedAt={turnStartedAt} /> : null}
            </View>
          );
        }
        case "note":
          return (
            <View style={styles.noteRow}>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{item.text}</Text>
            </View>
          );
        case "filler":
        default:
          return (
            <View style={styles.streamingRow}>
              <Text style={[typeScale.body, { color: tokens.ink2 }]}>{item.text}</Text>
            </View>
          );
      }
    },
    [tokens.ink2, tokens.ink3, tokens.selection, findSeq, busy, activity, activityToolLine, turnStartedAt, onMessageLongPress, openGroups, toggleGroup],
  );

  // Each row reports its height so the sticky prompt bar can tell which row is at the top.
  const renderItem = useCallback(
    ({ item }: { item: TimelineItem }) => (
      <View onLayout={(e) => onRowLayout(item.id, e.nativeEvent.layout.height)}>{renderRow(item)}</View>
    ),
    [renderRow, onRowLayout],
  );

  const keyExtractor = useCallback((item: TimelineItem) => item.id, []);

  const onEndReached = useCallback(() => {
    if (view.hasNextPage && !view.isFetchingNextPage) {
      void view.fetchNextPage();
    }
  }, [view]);

  const queued = snapshot?.queued ?? [];
  const hasPending = queued.length > 0 || offlineQueue.length > 0;
  const assistantResponseIndices = useMemo(() => items.reduce<number[]>((indices, item, index) => { if (item.kind === "history" && item.row.role === "assistant") indices.push(index); return indices; }, []), [items]);
  const [responsePosition, setResponsePosition] = useState(0);
  const [showResponseNav, setShowResponseNav] = useState(false);
  useEffect(() => { setResponsePosition((position) => Math.min(position, Math.max(0, assistantResponseIndices.length - 1))); }, [assistantResponseIndices.length]);
  const jumpToPreviousResponse = useCallback(() => {
    if (assistantResponseIndices.length === 0) return;
    const previous = Math.min(assistantResponseIndices.length - 1, responsePosition + 1);
    setResponsePosition(previous);
    listRef.current?.scrollToIndex({ index: assistantResponseIndices[previous], animated: true, viewPosition: 0.5 });
  }, [assistantResponseIndices, responsePosition]);
  const jumpToNextResponse = useCallback(() => {
    if (assistantResponseIndices.length === 0) return;
    const next = Math.max(0, responsePosition - 1);
    setResponsePosition(next);
    listRef.current?.scrollToIndex({ index: assistantResponseIndices[next], animated: true, viewPosition: 0.5 });
  }, [assistantResponseIndices, responsePosition]);
  // On a phone the response stepper and the jump pill stacked two floating controls over the
  // reading area; the jump button alone stays there.
  const { isCompact, isExpanded } = useBreakpoint();
  const finePointer = useFinePointer();
  const promptEntries = useMemo<PromptEntry[]>(
    () =>
      items.flatMap((item, index) =>
        item.kind === "history" && item.row.role === "user" && item.row.kind !== "nudge" && item.row.visibility === "llm"
          ? [{ index, seq: item.row.seq, text: displayMessageText(item.row) }]
          : [],
      ),
    [items],
  );
  const topIndex = topRow.top;
  const stickyPrompt = topIndex != null ? stickyPromptFor(promptEntries, topIndex) : null;
  const scrollToSticky = useCallback(() => {
    if (!stickyPrompt) return;
    try {
      listRef.current?.scrollToIndex({ index: stickyPrompt.index, animated: true, viewPosition: 0.1 });
    } catch {
      requestMessageJump(sessionId, stickyPrompt.seq);
    }
  }, [stickyPrompt, sessionId]);
  const rail = usePromptRail(baseUrl, sessionId, historyRows, isExpanded && finePointer);
  const showRail = railVisible({ promptCount: rail.total, expanded: isExpanded, finePointer });
  const responseNavigatorVisible = !isCompact && showResponseNav && assistantResponseIndices.length > 1;

  return (
    // "bottom" deliberately omitted: Composer owns the home-indicator inset itself (its bg2
    // panel bleeds through it) so Screen's bg1 never shows as a seam below the composer.
    <AgentIndexProvider value={agentIndex}>
    <Screen edges={["left", "right"]} keyboardAvoiding keyboardVerticalOffset={headerHeight} contentContainerStyle={styles.sessionColumn}>
      <View style={styles.flex} {...findFocusProps("chat")}>
        <BoundedList<TimelineItem>
          ref={listRef}
          // Only invert when there's content — an inverted FlatList mirrors its
          // ListEmptyComponent upside-down, so the empty state must render upright. Withheld
          // (see transcriptMountGate.ts) while `!transcriptReady`, same as `data` below.
          inverted={transcriptReady && items.length > 0}
          data={transcriptReady ? items : EMPTY_TIMELINE}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          onScroll={onScroll}
          onScrollToIndexFailed={onScrollToIndexFailed}
          onLayout={(e) => {
            autofill.onLayout(e);
            onViewport(e.nativeEvent.layout.height);
          }}
          onContentSizeChange={autofill.onContentSizeChange}
          scrollEventThrottle={32}
          onEndReached={onEndReached}
          loadingMore={transcriptReady && view.isFetchingNextPage}
          ListEmptyComponent={
            !transcriptReady || (snapshot == null && !snapshotTimedOut) || (!historySettled && !historyQuery.isError) ? (
              <View style={styles.connectingRow}>
                <ActivityIndicator size="small" color={tokens.accent} />
                <Text style={[typeScale.meta, { color: tokens.ink3 }]}>
                  {snapshot == null ? "connecting to session…" : "loading messages…"}
                </Text>
              </View>
            ) : historyDead ? (
              <EmptyState
                icon={historyErrorStatus === 404 ? SearchX : WifiOff}
                message={
                  historyErrorStatus === 404
                    ? "session not found on this server"
                    : "couldn't load this session — check the server connection"
                }
              />
            ) : historyEmpty ? (
              <EmptyState icon={MessageSquare} message="no messages yet — say something to get started" />
            ) : (
              <EmptyState icon={WifiOff} message="couldn't load this session's messages" />
            )
          }
        />

        {stickyPrompt ? <StickyPromptBar text={stickyPrompt.text} onPress={scrollToSticky} /> : null}
        {showRail ? (
          <PromptRail
            ticks={rail.ticks}
            activeSeq={stickyPrompt?.seq ?? null}
            onSelect={(seq) => requestMessageJump(sessionId, seq)}
          />
        ) : null}
        <FindBar targetId="chat" />
        {responseNavigatorVisible ? (
          <View style={[styles.responseNav, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}>
            <Pressable
              onPress={jumpToPreviousResponse}
              disabled={responsePosition === assistantResponseIndices.length - 1}
              accessibilityRole="button"
              accessibilityLabel="Previous assistant response"
              style={styles.responseButton}
            >
              <ChevronUp size={16} color={tokens.ink2} />
            </Pressable>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{`${responsePosition + 1} / ${assistantResponseIndices.length} responses`}</Text>
            <Pressable
              onPress={jumpToNextResponse}
              disabled={responsePosition === 0}
              accessibilityRole="button"
              accessibilityLabel="Next assistant response"
              style={styles.responseButton}
            >
              <ChevronDown size={16} color={tokens.ink2} />
            </Pressable>
          </View>
        ) : null}
        {showJump || view.detached ? (
          <Pressable
            onPress={jumpToLatest}
            accessibilityRole="button"
            accessibilityLabel="Return to latest"
            style={[styles.jumpPill, { backgroundColor: busy ? tokens.selection : tokens.bg3, borderColor: busy ? tokens.accent : tokens.border }]}
          >
            <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink2} />
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{newWhileAway > 0 ? `${newWhileAway} new` : busy ? "Back to live" : "Latest"}</Text>
          </Pressable>
        ) : null}
      </View>

      {hasPending ? (
        <QueueStrip
          queued={queued}
          offline={offlineQueue}
          online={online}
          canReorder={followUp.supported}
          onOfflineChange={setOfflineQueue}
          onDropped={(text) => setPendingSent((prev) => prev.filter((p) => p.text !== text))}
        />
      ) : null}

      <CardSlot />

      <IntegrityDocks baseUrl={baseUrl} sessionId={sessionId} snapshot={snapshot} />

      <SubagentStrip
        subagents={snapshot?.subagents ?? []}
        onPress={() => paneNav.openAgents(sessionId)}
        onOpenAgent={paneNav.openSession}
      />

      <ComposerDocks sessionId={sessionId} snapshot={snapshot} />

      {readOnly ? (
        // No input path at all reaches this session — never mount the interactive composer
        // (send/interrupt) for one. The daemon refuses the WS input path server-side too
        // (`reject_ws_if_local_read_only` in `serve.rs`); this is the UI half of that same "no
        // command/prompt/interrupt/kill path" contract, not just cosmetic.
        <View style={styles.readOnlyBar} accessibilityRole="text">
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
            {parentSession != null
              ? "subagent — view only, it works for its parent session"
              : connectionAccess.viewOnly
                ? "This connection can view but not change"
                : "read-only — running in a terminal, view only"}
          </Text>
        </View>
      ) : (
        <>
        <DeadPinBanner model={snapshot?.model} pinned={snapshot?.model_pinned} onReplace={(id) => handleSend(`/model ${id}`, [])} />
        <Composer
          sessionId={sessionId}
          busy={snapshot?.busy ?? false}
          online={online}
          suggestedPrompt={snapshot?.suggested_prompt ?? null}
          model={snapshot?.model ?? null}
          modelPinned={snapshot?.model_pinned}
          effort={snapshot?.effort ?? null}
          temper={snapshot?.temper ?? null}
          onSend={handleSend}
          recallPrompts={recallPrompts}
          onInterrupt={handleInterrupt}
        />
        </>
      )}
      {actionsAnchor != null ? (
        <MessageActionsMenu
          message={selectedMessage}
          anchor={actionsAnchor}
          onClose={() => {
            setSelectedMessage(null);
            setActionsAnchor(null);
          }}
          onQuote={onQuote}
          onEdit={onEditMessage}
          onRewind={onRewind}
          rewindDisabledReason={rewindBlocked}
          rewindWarning={rewindWarning}
        />
      ) : (
        <MessageActionsSheet
          visible={selectedMessage !== null}
          message={selectedMessage}
          onClose={() => setSelectedMessage(null)}
          onQuote={onQuote}
          onEdit={onEditMessage}
          onRewind={onRewind}
          rewindDisabledReason={rewindBlocked}
          rewindWarning={rewindWarning}
        />
      )}
    </Screen>
    </AgentIndexProvider>
  );
}

const styles = StyleSheet.create({
  // Hearth desktop/web: the chat column caps at 760px, centered in the remaining Fleet+Session
  // pane (a no-op at mobile widths — see prototype "Desktop Fleet + Session").
  // Full available width on every surface (user call: the capped 760px column wasted
  // desktop space); message rows keep their own gutters.
  sessionColumn: { width: "100%" },
  flex: { flex: 1 },
  readOnlyBar: {
    alignItems: "center",
    justifyContent: "center",
    paddingVertical: space.space12,
    paddingHorizontal: space.space16,
  },
  streamingRow: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
  connectingRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    paddingVertical: space.space24,
  },
  noteRow: { paddingHorizontal: space.space16, paddingVertical: space.space4 },
  jumpPill: {
    position: "absolute",
    bottom: space.space16,
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    minWidth: 44,
    minHeight: 44,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
  },
  responseNav: { position: "absolute", bottom: 68, alignSelf: "center", minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space4, borderRadius: 999, borderWidth: StyleSheet.hairlineWidth },
  responseButton: { width: 36, height: 36, alignItems: "center", justifyContent: "center" },
});

// Desktop/web: the session layout keeps this segment mounted itself (SegmentKeepAlive), so the
// route renders nothing there; phones render the screen as the route.
export default function SessionChatRoute() {
  return useSegmentsHosted() ? null : <SessionChat />;
}
