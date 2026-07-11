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
import AsyncStorage from "@react-native-async-storage/async-storage";
import { ChevronDown, Clock, Hammer, MessageSquare, SearchX, WifiOff } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  FlatList,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withSequence,
  withTiming,
} from "react-native-reanimated";

import type { SentAttachment } from "../../../components/chat/attach";
import CardSlot from "../../../components/chat/CardSlot";
import { Composer } from "../../../components/chat/Composer";
import { Markdown } from "../../../components/chat/Markdown";
import { MessageRow } from "../../../components/chat/MessageRow";
import { ReasoningDisclosure } from "../../../components/chat/ReasoningDisclosure";
import { BoundedList } from "../../../components/ds/BoundedList";
import { Chip } from "../../../components/ds/Chip";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { useToast } from "../../../components/ds/ToastHost";
import { type HistoryRow } from "../../../lib/api";
import { haptics } from "../../../lib/haptics";
import { useHistory } from "../../../lib/queries";
import { parseReasoning } from "../../../lib/reasoning";
import { useSessionCtx } from "../../../lib/sessionContext";
import { easings, useEmberdot, useForgeline } from "../../../theme/motion";
import { useTokens } from "../../../theme/ThemeProvider";
import { radii, space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";

// T3.3's CardSlot.tsx landed during this task (was a HANDOFF stub) — wired in directly above.

const OFFLINE_QUEUE_PREFIX = "forge.offlineQueue";
const OFFLINE_QUEUE_CAP = 20;
const JUMP_THRESHOLD_PX = 240;

function offlineQueueKey(baseUrl: string | null, sessionId: string): string {
  return `${OFFLINE_QUEUE_PREFIX}:${baseUrl ?? "unknown"}:${sessionId}`;
}

type TimelineItem =
  | { kind: "streaming"; id: string; text: string; streaming: boolean }
  | { kind: "history"; id: string; row: HistoryRow }
  | { kind: "filler"; id: string; text: string }
  | { kind: "note"; id: string; text: string }
  | { kind: "pendingSent"; id: string; text: string; attachments: SentAttachment[] };

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

// Kindle streaming (DESIGN_SYSTEM.md §5.2), rendered through Markdown instead of a plain <Text>
// so finalize (swap to MessageRow, which also goes through Markdown) is a visual no-op instead
// of a "pop" from raw markdown to formatted. A local equivalent of StreamingText.tsx's
// coalescing+caret shell rather than an edit to that shared component. Markdown.tsx's block/
// inline parsers already degrade safely on partial input (verified by inspection): an unclosed
// ``` fence collects lines to EOF and still renders as an in-progress code block, an unmatched
// ** or _ just falls through to literal text — nothing here needs extra guarding against a
// mid-token markdown string.
const CARET_SIZE = 7;
const CARET_PULSE_MS = 1000;
const CARET_MIN_OPACITY = 0.4;

function formatElapsed(elapsedMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0 ? `${minutes}m ${seconds.toString().padStart(2, "0")}s` : `${seconds}s`;
}

function LiveToolActivity({ line, startedAt }: { line: string; startedAt: number }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot("busy");
  const entrance = useForgeline(0);
  const [elapsedMs, setElapsedMs] = useState(() => Date.now() - startedAt);

  useEffect(() => {
    setElapsedMs(Date.now() - startedAt);
    const timer = setInterval(() => setElapsedMs(Date.now() - startedAt), 1000);
    return () => clearInterval(timer);
  }, [startedAt]);

  return (
    <Animated.View style={[styles.activityRow, { backgroundColor: tokens.bg2, borderColor: tokens.border }, entrance]}>
      <Animated.View style={[styles.activityDot, { backgroundColor: tokens.accent }, dotStyle]} />
      <Hammer size={14} strokeWidth={1.75} color={tokens.accent} />
      <Text style={[typeScale.meta, styles.activityLine, { color: tokens.ink2 }]} numberOfLines={1}>
        {line}
      </Text>
      <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{formatElapsed(elapsedMs)}</Text>
    </Animated.View>
  );
}

function StreamingAnswer({ text, streaming }: { text: string; streaming: boolean }) {
  const tokens = useTokens();
  const reduced = useReducedMotion();

  // Same rAF coalescing as StreamingText.tsx: at most one committed render per frame while
  // streaming; reduce-motion commits every update instantly instead.
  const [displayText, setDisplayText] = useState(text);
  const latestRef = useRef(text);
  const rafRef = useRef<number | null>(null);

  useEffect(() => {
    latestRef.current = text;
    if (reduced) {
      setDisplayText(text);
      return;
    }
    if (rafRef.current == null) {
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        setDisplayText(latestRef.current);
      });
    }
  }, [text, reduced]);

  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    },
    [],
  );

  // Caret pulse while streaming — same timing as StreamingText.tsx's Kindle caret.
  const caretOpacity = useSharedValue(1);
  useEffect(() => {
    if (reduced || !streaming) {
      caretOpacity.value = 1;
      return;
    }
    caretOpacity.value = withRepeat(
      withSequence(
        withTiming(CARET_MIN_OPACITY, { duration: CARET_PULSE_MS / 2, easing: easings.standard }),
        withTiming(1, { duration: CARET_PULSE_MS / 2, easing: easings.standard }),
      ),
      -1,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streaming, reduced]);
  const caretStyle = useAnimatedStyle(() => ({ opacity: caretOpacity.value }));

  return (
    <View style={styles.streamingAnswerRow}>
      <View style={styles.streamingAnswerText}>
        <Markdown content={displayText} />
      </View>
      {streaming ? (
        <Animated.View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
          style={[styles.caret, { backgroundColor: tokens.accent }, caretStyle]}
        />
      ) : null}
    </View>
  );
}

export default function SessionChat() {
  const tokens = useTokens();
  const toast = useToast();
  const { sessionId, baseUrl, snapshot, snapshotTimedOut, connectionState, send, headerHeight } = useSessionCtx();

  const historyQuery = useHistory(sessionId);

  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [showJump, setShowJump] = useState(false);
  const showJumpRef = useRef(false);
  useEffect(() => {
    showJumpRef.current = showJump;
  }, [showJump]);

  // ---------------------------------------------------------------------
  // Offline prompt queue (ARCHITECTURE §4.2): per server+session AsyncStorage queue, cap 20,
  // flushed in order on the exact reconnect edge, rendered as "queued (offline)" chips above
  // the composer. Loud drop past the cap (toast + haptic, never a silent no-op).
  // ---------------------------------------------------------------------
  const [offlineQueue, setOfflineQueue] = useState<string[]>([]);
  const offlineLoadedRef = useRef(false);

  useEffect(() => {
    offlineLoadedRef.current = false;
    let cancelled = false;
    AsyncStorage.getItem(offlineQueueKey(baseUrl, sessionId)).then((raw) => {
      if (cancelled) return;
      if (raw) {
        try {
          const parsed: unknown = JSON.parse(raw);
          setOfflineQueue(Array.isArray(parsed) ? (parsed as string[]) : []);
        } catch {
          setOfflineQueue([]);
        }
      } else {
        setOfflineQueue([]);
      }
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
    if (offlineQueue.length > 0) {
      for (const text of offlineQueue) send({ kind: "prompt", text });
      setOfflineQueue([]);
    }
    // The pending interrupt is edge-only: it means "Stop pressed while offline", so it fires on the
    // reconnect edge, not on a plain remount-while-online.
    if (pendingInterrupt && was !== "open") {
      send({ kind: "interrupt" });
      setPendingInterrupt(false);
    }
  }, [connectionState, offlineQueue, pendingInterrupt, send]);

  const online = connectionState === "open";

  // Optimistic "sent" bubble (ARCHITECTURE §4.1.4 timeline is otherwise server-truth-only):
  // without this, the user's own message doesn't appear anywhere until the whole turn
  // completes and history is invalidated/refetched — the composer looked like it swallowed
  // the prompt. `latestSeqRef` mirrors `historyRows[0]?.seq` (computed further down) so
  // `handleSend`, defined here, can read its current value without reordering the file.
  const [pendingSent, setPendingSent] = useState<PendingSent[]>([]);
  const latestSeqRef = useRef(-1);

  const handleSend = useCallback(
    (text: string, attachments: SentAttachment[]) => {
      const id = `p${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      setPendingSent((prev) => [...prev, { id, text, baselineSeq: latestSeqRef.current, attachments }]);

      // Only uploaded (path-bearing) attachments can ride the wire — correlate this exact send's
      // attachments to this exact prompt (ARCHITECTURE §4.1.4 / mobile upload race).
      const withPath = attachments
        .filter((a): a is SentAttachment & { path: string } => a.path != null)
        .map((a) => ({ path: a.path, image: a.image }));
      if (send({ kind: "prompt", text, attachments: withPath })) return;
      // Socket closed between the `online` render and here (race window). Fall through to
      // the offline queue — it will be flushed in order on the next !open→open edge.
      setOfflineQueue((prev) => {
        if (prev.length >= OFFLINE_QUEUE_CAP) {
          toast.show("offline queue full (20) — prompt dropped", { tone: "danger" });
          haptics.mergeConflict();
          setPendingSent((p) => p.filter((x) => x.id !== id));
          return prev;
        }
        return [...prev, text];
      });
    },
    [online, send, toast],
  );

  const handleInterrupt = useCallback(() => {
    // Try to send immediately; if the socket is closed, latch a pending interrupt that the
    // !open→open flush effect will deliver the moment the connection is restored.
    if (!send({ kind: "interrupt" })) setPendingInterrupt(true);
  }, [send]);

  const handleDequeue = useCallback(
    (index: number, text: string) => {
      if (!online) return;
      send({ kind: "dequeue", index, text });
      haptics.select();
    },
    [online, send],
  );

  const removeQueuedOffline = useCallback((index: number) => {
    setOfflineQueue((prev) => prev.filter((_, i) => i !== index));
    haptics.select();
  }, []);

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

  useEffect(() => {
    latestSeqRef.current = historyRows[0]?.seq ?? -1;
  }, [historyRows]);

  // Clear pendingSent bubbles once a real history row has landed since they were sent (turn
  // completed, ARCHITECTURE §4.1.4 invalidation) — same baselineSeq idea as `finalizing` below.
  useEffect(() => {
    if (pendingSent.length === 0) return;
    const newestSeq = historyRows[0]?.seq ?? -1;
    setPendingSent((prev) => prev.filter((p) => p.baselineSeq === newestSeq));
  }, [historyRows, pendingSent.length]);

  // Safety net: never let a pendingSent bubble linger forever if history never advances for it
  // (session closed mid-turn, connection never came back, ...). A normal successful send clears
  // its timer here too (the effect above already removed it from `pendingSent`, which reruns
  // this effect and cancels the stale timer before it can fire) — so the toast only ever fires
  // for the genuine stuck case, not on every ordinary turn.
  useEffect(() => {
    if (pendingSent.length === 0) return;
    const timers = pendingSent.map((p) =>
      setTimeout(() => {
        setPendingSent((prev) => prev.filter((x) => x.id !== p.id));
        toast.show("message didn't confirm — check connection", { tone: "danger" });
      }, PENDING_SENT_TIMEOUT_MS),
    );
    return () => timers.forEach(clearTimeout);
  }, [pendingSent, toast]);

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

  // Clear once the finalized row has actually arrived (state cleanup only — see comment above).
  useEffect(() => {
    if (!finalizing) return;
    if ((historyRows[0]?.seq ?? -1) !== finalizing.baselineSeq) {
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

  const finalizingActive =
    !busy && finalizing !== null && (historyRows[0]?.seq ?? -1) === finalizing.baselineSeq;
  // Bridge the mid-busy empty tick too (root cause of the residual flicker): the daemon flushes
  // the reply out of `streaming` into `transcript` one or more frames BEFORE `busy` flips false
  // (verified live — `streaming` goes "" while still busy). Falling back to `track.retainedText`
  // while busy keeps the just-streamed answer on screen through those frames instead of blanking
  // it back to the "thinking…" indicator, then `finalizing` carries it across the busy->false
  // edge until the history row lands. No frame ever renders the answer absent.
  const displayText =
    streamingText || (busy ? track.retainedText : "") || (finalizingActive ? finalizing!.text : "");

  const latestToolLine = useMemo(
    () =>
      [...(snapshot?.transcript ?? [])]
        .reverse()
        .find((line) => line.trimStart().startsWith("↳ ")) ?? null,
    [snapshot?.transcript],
  );
  const [toolActivity, setToolActivity] = useState<{ line: string; startedAt: number } | null>(null);
  useEffect(() => {
    if (!busy || !latestToolLine) {
      setToolActivity(null);
      return;
    }
    setToolActivity((current) =>
      current?.line === latestToolLine ? current : { line: latestToolLine, startedAt: Date.now() },
    );
  }, [busy, latestToolLine]);

  const items = useMemo<TimelineItem[]>(() => {
    const list: TimelineItem[] = [];
    // `busy` alone (before any tokens arrive) still gets a "streaming" slot — rendered with
    // empty text as the thinking indicator below — so there's never a silent gap between
    // submit and the first token (previously nothing rendered here at all until `displayText`
    // was non-empty, which read as "stuck").
    if (displayText || busy) {
      list.push({ kind: "streaming", id: "streaming", text: displayText, streaming: busy || Boolean(streamingText) });
    }
    // Notes (turn-end notices, e.g. "compacting", "turn failed") are the NEWEST events — they must
    // sit at the newest slot (just under the streaming reply / above the composer in the inverted
    // list), not be pushed to the far/oldest end where they'd float above all history with no cue.
    const notes = snapshot?.notes ?? [];
    for (let i = notes.length - 1; i >= 0; i--) {
      list.push({ kind: "note", id: `n${i}`, text: notes[i] });
    }
    // Newest-first (inverted list): the user's own just-sent message is more recent than any
    // settled history row but older than the in-progress reply above, and later sends are newer
    // than earlier ones — walk pendingSent back-to-front.
    for (let i = pendingSent.length - 1; i >= 0; i--) {
      const p = pendingSent[i];
      list.push({ kind: "pendingSent", id: p.id, text: p.text, attachments: p.attachments });
    }
    if (!useTranscriptFiller) {
      for (const row of historyRows) {
        list.push({ kind: "history", id: `h${row.seq}`, row });
      }
    } else {
      // Snapshot transcript is chronological (oldest->newest); walk it back-to-front for the
      // inverted list. It remains visible after an empty history response because that response
      // cannot prove the server transcript is empty.
      const filler = snapshot?.transcript ?? [];
      for (let i = filler.length - 1; i >= 0; i--) {
        list.push({ kind: "filler", id: `f${i}`, text: filler[i] });
      }
    }
    return list;
  }, [displayText, streamingText, busy, pendingSent, useTranscriptFiller, historyRows, snapshot?.transcript, snapshot?.notes]);

  // Pin to the latest item when a NEW item lands at the newest slot — not on every streaming
  // text tick (same item id, StreamingAnswer owns its own rAF coalescing), and not when an older
  // page is appended at the far end (that only changes the last item, not the first).
  const newestKeyRef = useRef<string | null>(null);
  useEffect(() => {
    const newestKey = items[0]?.id ?? null;
    if (newestKey !== newestKeyRef.current) {
      newestKeyRef.current = newestKey;
      if (!showJumpRef.current) {
        listRef.current?.scrollToOffset({ offset: 0, animated: false });
      }
    }
  }, [items]);

  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    setShowJump(e.nativeEvent.contentOffset.y > JUMP_THRESHOLD_PX);
  }, []);

  const jumpToLatest = useCallback(() => {
    listRef.current?.scrollToOffset({ offset: 0, animated: true });
  }, []);

  const renderItem = useCallback(
    ({ item }: { item: TimelineItem }) => {
      switch (item.kind) {
        case "history":
          return <MessageRow row={item.row} />;
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
            />
          );
        case "streaming": {
          // Split inline `<think>…</think>` out of the live text: reasoning goes to the collapsed
          // disclosure, only the answer streams in the main slot (never the raw thinking log).
          const parsed = parseReasoning(item.text);
          const hasReasoning = parsed.reasoning.length > 0 || parsed.thinking;
          if (!hasReasoning && !parsed.answer) {
            // Queued/busy but no answer/reasoning yet — an explicit "thinking" affordance (same
            // ActivityIndicator+accent pairing BoundedList's loadingMore footer uses) so this
            // phase never reads as stuck.
            return item.streaming ? (
              toolActivity ? (
                <View style={styles.streamingRow}>
                  <LiveToolActivity line={toolActivity.line} startedAt={toolActivity.startedAt} />
                </View>
              ) : (
                <View style={[styles.streamingRow, styles.thinkingRow]}>
                  <ActivityIndicator size="small" color={tokens.accent} />
                  <Text style={[typeScale.meta, { color: tokens.ink3 }]}>thinking…</Text>
                </View>
              )
            ) : null;
          }
          return (
            <View style={styles.streamingRow}>
              {hasReasoning ? (
                <ReasoningDisclosure
                  reasoning={parsed.reasoning}
                  active={parsed.thinking && item.streaming}
                />
              ) : null}
              {parsed.answer ? (
                <StreamingAnswer text={parsed.answer} streaming={item.streaming} />
              ) : null}
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
    [tokens.ink2, tokens.ink3, tokens.accent, toolActivity],
  );

  const keyExtractor = useCallback((item: TimelineItem) => item.id, []);

  const onEndReached = useCallback(() => {
    if (historyQuery.hasNextPage && !historyQuery.isFetchingNextPage) {
      void historyQuery.fetchNextPage();
    }
  }, [historyQuery]);

  const queued = snapshot?.queued ?? [];
  const hasPending = queued.length > 0 || offlineQueue.length > 0;

  return (
    // "bottom" deliberately omitted: Composer owns the home-indicator inset itself (its bg2
    // panel bleeds through it) so Screen's bg1 never shows as a seam below the composer.
    <Screen edges={["left", "right"]} keyboardAvoiding keyboardVerticalOffset={headerHeight}>
      <View style={styles.flex}>
        <BoundedList<TimelineItem>
          ref={listRef}
          // Only invert when there's content — an inverted FlatList mirrors its
          // ListEmptyComponent upside-down, so the empty state must render upright.
          inverted={items.length > 0}
          data={items}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          onScroll={onScroll}
          scrollEventThrottle={32}
          onEndReached={onEndReached}
          loadingMore={historyQuery.isFetchingNextPage}
          ListEmptyComponent={
            (snapshot == null && !snapshotTimedOut) || (!historySettled && !historyQuery.isError) ? (
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

        {showJump ? (
          <Pressable
            onPress={jumpToLatest}
            accessibilityRole="button"
            accessibilityLabel="jump to latest"
            style={[styles.jumpPill, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}
          >
            <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink2} />
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>latest</Text>
          </Pressable>
        ) : null}
      </View>

      {hasPending ? (
        <View style={[styles.pendingRow, { borderTopColor: tokens.border }]}>
          {/* Tapping a queued chip sends `dequeue` (index+text echoed) so the daemon can reject a
           * stale tap after the queue shifted. */}
          {queued.map((text, i) => (
            <Chip key={`q${i}`} label={text} onPress={() => handleDequeue(i, text)} />
          ))}
          {offlineQueue.map((text, i) => (
            // Deliberately NOT `selected` (ember/accent) — this is a normal "will send on
            // reconnect" queue state, not an error, and the message itself already rendered as
            // a normal sent bubble via `pendingSent` above. Calm ink3 + a clock glyph instead.
            <Chip
              key={`o${i}`}
              label={`${text} (offline)`}
              icon={<Clock size={13} strokeWidth={1.75} color={tokens.ink3} />}
              onPress={() => removeQueuedOffline(i)}
            />
          ))}
        </View>
      ) : null}

      <CardSlot />

      <Composer
        sessionId={sessionId}
        busy={snapshot?.busy ?? false}
        online={online}
        onSend={handleSend}
        onInterrupt={handleInterrupt}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  streamingRow: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
  thinkingRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  activityRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    borderRadius: radii.radius8,
    borderWidth: StyleSheet.hairlineWidth,
  },
  activityDot: { width: 8, height: 8, borderRadius: 4 },
  activityLine: { flex: 1 },
  noteRow: { paddingHorizontal: space.space16, paddingVertical: space.space4 },
  connectingRow: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space8 },
  streamingAnswerRow: { flexDirection: "row", alignItems: "flex-end", flexWrap: "wrap" },
  streamingAnswerText: { flexShrink: 1 },
  caret: {
    width: CARET_SIZE,
    height: CARET_SIZE,
    borderRadius: CARET_SIZE / 2,
    marginLeft: 4,
    marginBottom: 4,
  },
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
  pendingRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingTop: space.space8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
});
