// Chat (T3.2). ARCHITECTURE.md §4.1.4 (CRITICAL — timeline source-of-truth + dedupe), §4.2
// (offline prompt queue). FEATURES.md §1.2/§1.3 (Snapshot fields / RemoteInput -> UI).
// DESIGN_SYSTEM.md §6 (PromptComposer, MessageRow, Kindle streaming).
//
// §4.1.4 dedupe rule, implemented literally (not content-matched): the timeline's source of
// truth is `useHistory` rows. The snapshot contributes ONLY (a) the live `streaming` edge while
// `busy`, always rendered, and (b) `transcript` tail lines as instant warm-start filler — used
// ONLY until `historyQuery.data` resolves for the first time for this session (from persisted
// cache or network), then dropped for good. The two sources (history vs. filler) are mutually
// exclusive per render, so there is nothing to de-duplicate by content — they simply never
// co-exist. Turn-completion history invalidation (busy true->false) is already wired by the
// T3.1 session shell's `useTurnCompleted(snapshot)` call in `_layout.tsx` — not repeated here.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { ChevronDown, MessageSquare } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  FlatList,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";

import CardSlot from "../../../components/chat/CardSlot";
import { Composer } from "../../../components/chat/Composer";
import { MessageRow } from "../../../components/chat/MessageRow";
import { StreamingText } from "../../../components/chat/StreamingText";
import { BoundedList } from "../../../components/ds/BoundedList";
import { Chip } from "../../../components/ds/Chip";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Screen } from "../../../components/ds/Screen";
import { useToast } from "../../../components/ds/ToastHost";
import { type HistoryRow } from "../../../lib/api";
import { haptics } from "../../../lib/haptics";
import { useHistory } from "../../../lib/queries";
import { useSessionCtx } from "../../../lib/sessionContext";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";

// T3.3's CardSlot.tsx landed during this task (was a HANDOFF stub) — wired in directly above.

const OFFLINE_QUEUE_PREFIX = "forge.offlineQueue";
const OFFLINE_QUEUE_CAP = 20;
const JUMP_THRESHOLD_PX = 240;

function offlineQueueKey(baseUrl: string | null, sessionId: string): string {
  return `${OFFLINE_QUEUE_PREFIX}:${baseUrl ?? "unknown"}:${sessionId}`;
}

type TimelineItem =
  | { kind: "streaming"; id: string; text: string }
  | { kind: "history"; id: string; row: HistoryRow }
  | { kind: "filler"; id: string; text: string };

export default function SessionChat() {
  const tokens = useTokens();
  const toast = useToast();
  const { sessionId, baseUrl, snapshot, connectionState, send } = useSessionCtx();

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

  // Flush in order on the exact `!open -> open` edge (not on every offlineQueue change).
  const prevConnRef = useRef(connectionState);
  useEffect(() => {
    const was = prevConnRef.current;
    prevConnRef.current = connectionState;
    if (was !== "open" && connectionState === "open" && offlineQueue.length > 0) {
      for (const text of offlineQueue) send({ kind: "prompt", text });
      setOfflineQueue([]);
    }
  }, [connectionState, offlineQueue, send]);

  const online = connectionState === "open";

  const handleSend = useCallback(
    (text: string) => {
      if (online) {
        send({ kind: "prompt", text });
        return;
      }
      setOfflineQueue((prev) => {
        if (prev.length >= OFFLINE_QUEUE_CAP) {
          toast.show("offline queue full (20) — prompt dropped", { tone: "danger" });
          haptics.mergeConflict();
          return prev;
        }
        return [...prev, text];
      });
    },
    [online, send, toast],
  );

  const handleInterrupt = useCallback(() => {
    send({ kind: "interrupt" });
  }, [send]);

  const removeQueuedOffline = useCallback((index: number) => {
    setOfflineQueue((prev) => prev.filter((_, i) => i !== index));
    haptics.select();
  }, []);

  // ---------------------------------------------------------------------
  // Timeline construction (ARCHITECTURE §4.1.4)
  // ---------------------------------------------------------------------
  const historyRows = useMemo<HistoryRow[]>(() => {
    const pages = historyQuery.data?.pages ?? [];
    return pages.flat().filter((r) => r.visibility === "ui");
  }, [historyQuery.data]);

  // Once `data` has resolved once (cache or network) for this session, the filler is gone for
  // good — never re-armed by a later refetch/invalidation.
  const historySettled = historyQuery.data !== undefined;

  const streamingText = snapshot?.busy ? snapshot.streaming : "";

  const items = useMemo<TimelineItem[]>(() => {
    const list: TimelineItem[] = [];
    if (streamingText) {
      list.push({ kind: "streaming", id: "streaming", text: streamingText });
    }
    if (historySettled) {
      for (const row of historyRows) {
        list.push({ kind: "history", id: `h${row.seq}`, row });
      }
    } else {
      // Warm-start filler only: `transcript` is a chronological (oldest->newest) scrollback
      // tail, so walk it back-to-front to match the inverted list's newest-first data order.
      const filler = snapshot?.transcript ?? [];
      for (let i = filler.length - 1; i >= 0; i--) {
        list.push({ kind: "filler", id: `f${i}`, text: filler[i] });
      }
    }
    return list;
  }, [streamingText, historySettled, historyRows, snapshot?.transcript]);

  // Pin to the latest item when a NEW item lands at the newest slot — not on every streaming
  // text tick (same item id, StreamingText owns its own rAF coalescing), and not when an older
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
        case "streaming":
          return (
            <View style={styles.streamingRow}>
              <StreamingText text={item.text} streaming />
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
    [tokens.ink2],
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
    <Screen edges={["left", "right", "bottom"]} keyboardAvoiding>
      <View style={styles.flex}>
        <BoundedList<TimelineItem>
          ref={listRef}
          inverted
          data={items}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          onScroll={onScroll}
          scrollEventThrottle={32}
          onEndReached={onEndReached}
          loadingMore={historyQuery.isFetchingNextPage}
          ListEmptyComponent={
            <EmptyState icon={MessageSquare} message="no messages yet — say something to get started" />
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
          {queued.map((text, i) => (
            <Chip key={`q${i}`} label={text} />
          ))}
          {offlineQueue.map((text, i) => (
            <Chip
              key={`o${i}`}
              label={`${text} (offline)`}
              selected
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
  jumpPill: {
    position: "absolute",
    bottom: space.space16,
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
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
