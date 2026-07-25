// Machined Replay (Mobile/Desktop "Session Review Replay"/"Session Replay" frames):
// chronological log rows (fixed-width mono timestamp + role columns) + a scrub bar with a
// thumb and an elapsed counter. Presentational — no wire coupling: the caller fetches
// `HistoryRow[]` via `useHistory(sessionId)` (see `app/session/[id]/replay.tsx` for the live
// wiring pattern) and hands the resolved rows + loading/error state to this view.
//
// Protocol v9 gave `/api/history` rows two fields this view runs on:
//   `kind`       — the row's real provenance in the transcript vocabulary, so the role column
//                  shows sys/you/forge/tool instead of being inferred from `role`.
//   `elapsed_ms` — offset from the session's FIRST visible row, i.e. a real zero point. The
//                  scrub counter reads mm:ss elapsed off it instead of the wall-clock
//                  approximation it used before.
// Both are absent from a pre-v9 daemon: `kind` falls back to `role`, and the scrub counter
// falls back to the old HH:MM/HH:MM clock pair. The per-row timestamp column stays wall-clock
// either way — it is the row's real `created_at`, and useful independently of the scrubber.
//
// The replay screen asks for `include_tools`, so `kind === "tool"` rows DO appear here, and each
// carries `tool_phase`: a "call" row's content is the arguments the model sent (rendered as a
// call, under the tool's name), a "result" row's is what came back (rendered as output, as every
// tool row was before). A row with no `tool_phase` — an older daemon, or a page fetched without
// tools — renders exactly as it did: as output. Nothing is inferred from the prose.
import { Clock, Route as RouteIcon } from "lucide-react-native";
import React, { useCallback, useRef, useState } from "react";
import {
  ActivityIndicator,
  type GestureResponderEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Platform,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import type { HistoryRow, TranscriptKind } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Markdown } from "../chat/Markdown";
import { SystemOutput } from "../chat/SystemOutput";

const TIMESTAMP_COL_WIDTH = 46;
// Fits the longest label ("forge") at the mono meta size, so every body below lines up on the
// same left edge regardless of which kind the row is.
const ROLE_COL_WIDTH = 38;

// How close to the bottom of the currently-loaded log (in px) before requesting the next
// (older) page — mirrors BoundedList's onEndReachedThreshold, just measured in pixels since
// this view drives its own ScrollView instead of a FlatList.
const LOAD_MORE_THRESHOLD_PX = 200;

/** The row's v9 `kind`, or the pre-v9 derivation from `role` (which has no "tool" member). */
export function kindOf(row: HistoryRow): TranscriptKind {
  return row.kind ?? row.role;
}

function kindLabel(kind: TranscriptKind): string {
  if (kind === "user") return "you";
  if (kind === "assistant") return "forge";
  if (kind === "tool") return "tool";
  return "sys";
}

function kindColor(kind: TranscriptKind, tokens: ReturnType<typeof useTokens>): string {
  if (kind === "user") return tokens.accent;
  if (kind === "assistant") return tokens.ink2;
  if (kind === "tool") return tokens.ink3;
  return tokens.ink4;
}

function formatClock(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** `elapsed_ms` as `mm:ss`, or `h:mm:ss` past an hour. Second-resolution by construction — the
 * daemon derives it from whole-second `created_at` values. */
export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const seconds = String(total % 60).padStart(2, "0");
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`;
  return `${String(minutes).padStart(2, "0")}:${seconds}`;
}

export interface ReplayViewProps {
  rows: HistoryRow[];
  loading?: boolean;
  error?: boolean;
  onRetry?: () => void;
  /** Pull-to-refresh (native) — mirrors BoundedList's `refreshing`/`onRefresh`. */
  refreshing?: boolean;
  onRefresh?: () => void;
  /** Called when the scroll position nears the end of the currently-loaded log — the
   * caller is responsible for guarding against redundant fetches (e.g. `hasNextPage &&
   * !isFetchingNextPage`), same contract as BoundedList's `onEndReached`. */
  onEndReached?: () => void;
  /** Shows a footer spinner while a next page is being fetched. */
  loadingMore?: boolean;
}

export function ReplayView({
  rows,
  loading = false,
  error = false,
  onRetry,
  refreshing = false,
  onRefresh,
  onEndReached,
  loadingMore = false,
}: ReplayViewProps) {
  const tokens = useTokens();
  const scrollRef = useRef<ScrollView>(null);
  const contentHeight = useRef(0);
  const viewportHeight = useRef(0);
  const [progress, setProgress] = useState(0); // 0..1 scroll fraction

  const onContentSizeChange = useCallback((_w: number, h: number) => {
    contentHeight.current = h;
  }, []);
  const onLayout = useCallback((h: number) => {
    viewportHeight.current = h;
  }, []);
  const onScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const max = Math.max(1, contentHeight.current - viewportHeight.current);
      const y = e.nativeEvent.contentOffset.y;
      setProgress(Math.max(0, Math.min(1, y / max)));
      if (onEndReached && !loadingMore) {
        const distanceFromEnd = contentHeight.current - viewportHeight.current - y;
        if (distanceFromEnd < LOAD_MORE_THRESHOLD_PX) onEndReached();
      }
    },
    [onEndReached, loadingMore],
  );

  const seekTo = useCallback((fraction: number) => {
    const clamped = Math.max(0, Math.min(1, fraction));
    const max = Math.max(0, contentHeight.current - viewportHeight.current);
    scrollRef.current?.scrollTo({ y: clamped * max, animated: false });
    setProgress(clamped);
  }, []);

  const [trackWidth, setTrackWidth] = useState(0);
  const seekAtEvent = useCallback(
    (e: GestureResponderEvent) => seekTo(e.nativeEvent.locationX / Math.max(1, trackWidth)),
    [seekTo, trackWidth],
  );

  const currentIndex = rows.length > 0 ? Math.min(rows.length - 1, Math.round(progress * (rows.length - 1))) : 0;
  // Elapsed is the real counter whenever the daemon supplies it for BOTH ends of the range —
  // a v8 host (no field) or an unreadable epoch (null) falls back to the wall-clock pair.
  const endElapsed = rows.length > 0 ? rows[rows.length - 1].elapsed_ms : null;
  const currentElapsed = rows.length > 0 ? rows[currentIndex].elapsed_ms : null;
  const scrubCounter =
    endElapsed != null && currentElapsed != null
      ? `${formatElapsed(currentElapsed)}/${formatElapsed(endElapsed)}`
      : rows.length > 0
        ? `${formatClock(rows[currentIndex].created_at)}/${formatClock(rows[rows.length - 1].created_at)}`
        : "";

  if (loading) {
    return (
      <View style={styles.state}>
        <ActivityIndicator color={tokens.accent} />
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Loading replay…</Text>
      </View>
    );
  }
  if (error) {
    return <EmptyState icon={Clock} message="Could not load this replay." action={onRetry ? <RetryLink onPress={onRetry} /> : undefined} />;
  }
  if (rows.length === 0) {
    return <EmptyState icon={RouteIcon} message="No saved messages yet." />;
  }

  return (
    <View style={styles.root}>
      <ScrollView
        ref={scrollRef}
        style={styles.log}
        contentContainerStyle={styles.logContent}
        onContentSizeChange={onContentSizeChange}
        onLayout={(e) => onLayout(e.nativeEvent.layout.height)}
        onScroll={onScroll}
        scrollEventThrottle={32}
        refreshControl={
          Platform.OS !== "web" && onRefresh ? (
            <RefreshControl
              refreshing={refreshing}
              onRefresh={onRefresh}
              tintColor={Platform.OS === "ios" ? "transparent" : tokens.accent}
              colors={[tokens.accent]}
            />
          ) : undefined
        }
      >
        {/* `seq` is NOT unique on a tools page: an assistant carrier's synthesized call rows all
            carry the carrier's own seq, so the position has to join the key. */}
        {rows.map((row, index) => (
          <ReplayRow key={`${row.seq}-${index}`} row={row} showSeparator={index < rows.length - 1} />
        ))}
        {loadingMore ? (
          <View style={styles.loadingMore}>
            <ActivityIndicator color={tokens.accent} />
          </View>
        ) : null}
      </ScrollView>

      {rows.length > 1 ? (
        <View style={styles.scrubWrap}>
          <View
            style={[styles.scrubTrack, { backgroundColor: tokens.border }]}
            onLayout={(e) => setTrackWidth(e.nativeEvent.layout.width)}
            onStartShouldSetResponder={() => true}
            onMoveShouldSetResponder={() => true}
            onResponderGrant={seekAtEvent}
            onResponderMove={seekAtEvent}
          >
            <View style={[styles.scrubFill, { width: `${progress * 100}%`, backgroundColor: tokens.ink3 }]} />
            <View
              style={[
                styles.scrubThumb,
                { left: `${progress * 100}%`, backgroundColor: tokens.ink, borderColor: tokens.bg0 },
              ]}
              pointerEvents="none"
            />
          </View>
          <Text style={[typeScale.monoMeta, tabularNums, styles.scrubClock, { color: tokens.ink4 }]}>
            {scrubCounter}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

function RetryLink({ onPress }: { onPress: () => void }) {
  const tokens = useTokens();
  return (
    <Text onPress={onPress} accessibilityRole="button" style={[typeScale.bodyBold, { color: tokens.accent }]}>
      Retry
    </Text>
  );
}

const ReplayRow = React.memo(function ReplayRow({ row, showSeparator }: { row: HistoryRow; showSeparator: boolean }) {
  const tokens = useTokens();
  const kind = kindOf(row);
  const color = kindColor(kind, tokens);
  // Only a tool row has a phase to render; an absent one (older daemon, or a page fetched
  // without tools) means "unknown", which reads as the result rendering this view always used.
  const phase = kind === "tool" ? (row.tool_phase ?? null) : null;
  const isCall = phase === "call";
  // The arrow says which direction the row moved in — into the tool, or back out. Drawn only
  // when the daemon actually said which; a phase-less tool row gets its bare name.
  const marker = isCall ? "→" : phase === "result" ? "←" : null;
  const callArgs = isCall ? row.content.trim() : "";
  // A tool RESULT is machine output by definition, so it takes the boxed mono body; a system row
  // only does once it spans lines (a one-line notice reads better as plain sub text). A call
  // renders its own way below — its content is arguments, not output.
  const boxed = (kind === "tool" && !isCall) || (kind === "system" && row.content.includes("\n"));

  return (
    <View style={styles.entry}>
      <View style={styles.entryHead}>
        <Text style={[typeScale.monoMeta, tabularNums, styles.timestamp, { color: tokens.ink4 }]}>
          {formatClock(row.created_at)}
        </Text>
        <Text style={[typeScale.monoMeta, styles.role, { color }]} numberOfLines={1}>
          {kindLabel(kind)}
        </Text>
        {row.tool ? (
          <Text style={[typeScale.monoMeta, styles.toolName, { color: tokens.ink3 }]} numberOfLines={1}>
            {[marker, row.tool].filter(Boolean).join(" ")}
          </Text>
        ) : null}
      </View>
      {isCall ? (
        // Arguments, capped daemon-side. An empty summary renders nothing rather than an empty
        // box — the head line above already says which tool was called.
        callArgs.length > 0 ? (
          <View
            style={[
              styles.entryBody,
              styles.callArgs,
              { borderColor: tokens.border, backgroundColor: hexToRgba(tokens.accent, 0.06) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>{callArgs}</Text>
          </View>
        ) : null
      ) : boxed ? (
        <View style={styles.entryBody}>
          <SystemOutput content={row.content} />
        </View>
      ) : kind === "system" ? (
        <Text style={[typeScale.sub, styles.entryBody, { color: tokens.ink3 }]}>{row.content}</Text>
      ) : (
        <View style={styles.entryBody}>
          <Markdown content={row.content} />
        </View>
      )}
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
});

const styles = StyleSheet.create({
  root: { flex: 1 },
  state: { alignItems: "center", justifyContent: "center", padding: space.space32, gap: space.space12 },
  log: { flex: 1 },
  logContent: { paddingBottom: space.space16 },
  loadingMore: { alignItems: "center", paddingVertical: space.space16 },
  entry: { paddingTop: space.space16 },
  entryHead: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  timestamp: { width: TIMESTAMP_COL_WIDTH },
  role: { width: ROLE_COL_WIDTH, fontFamily: monoFamily.bold },
  toolName: { flex: 1 },
  entryBody: { marginTop: space.space8, marginLeft: TIMESTAMP_COL_WIDTH + space.space8 },
  callArgs: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space8 },
  separator: { height: StyleSheet.hairlineWidth, marginTop: space.space16 },
  scrubWrap: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingTop: space.space8 },
  scrubTrack: { flex: 1, height: 3, borderRadius: radii.radius4, position: "relative", justifyContent: "center" },
  scrubFill: { position: "absolute", left: 0, top: 0, bottom: 0, borderRadius: radii.radius4 },
  scrubThumb: {
    position: "absolute",
    width: 10,
    height: 10,
    marginLeft: -5,
    borderRadius: 5,
    borderWidth: 2,
  },
  scrubClock: { minWidth: 84, textAlign: "right" },
});
