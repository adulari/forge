// Machined Replay (Mobile/Desktop "Session Review Replay"/"Session Replay" frames):
// chronological log rows (sys/you/forge, fixed-width mono timestamp column) + a scrub bar
// with a thumb and a clock-time counter. Presentational — no wire coupling: the caller
// fetches `HistoryRow[]` via `useHistory(sessionId)` (see `app/session/[id]/replay.tsx` for
// the live wiring pattern) and hands the resolved rows + loading/error state to this view.
//
// Honesty: `HistoryRow.role` is only "user" | "assistant" | "system" — there is no distinct
// "tool" row on the wire, so this renders three kinds (you/forge/sys), not four. Timestamps
// are the row's real `created_at` (epoch seconds) rendered as local HH:MM — the design frame
// shows the same wall-clock format for its scrub counter ("09:13/09:41"), so the scrub bar
// mirrors that rather than inventing an elapsed mm:ss with no defined zero point.
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

import type { HistoryRow } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Markdown } from "../chat/Markdown";
import { SystemOutput } from "../chat/SystemOutput";

const TIMESTAMP_COL_WIDTH = 46;

// How close to the bottom of the currently-loaded log (in px) before requesting the next
// (older) page — mirrors BoundedList's onEndReachedThreshold, just measured in pixels since
// this view drives its own ScrollView instead of a FlatList.
const LOAD_MORE_THRESHOLD_PX = 200;

function roleLabel(role: HistoryRow["role"]): string {
  if (role === "user") return "you";
  if (role === "assistant") return "forge";
  return "sys";
}

function roleColor(role: HistoryRow["role"], tokens: ReturnType<typeof useTokens>): string {
  if (role === "user") return tokens.accent;
  if (role === "assistant") return tokens.ink2;
  return tokens.ink4;
}

function formatClock(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
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
  const endClock = rows.length > 0 ? formatClock(rows[rows.length - 1].created_at) : null;
  const currentClock = rows.length > 0 ? formatClock(rows[currentIndex].created_at) : null;

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
        {rows.map((row, index) => (
          <ReplayRow key={row.seq} row={row} showSeparator={index < rows.length - 1} />
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
            {`${currentClock}/${endClock}`}
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
  const color = roleColor(row.role, tokens);
  const multiline = row.role === "system" && row.content.includes("\n");

  return (
    <View style={styles.entry}>
      <View style={styles.entryHead}>
        <Text style={[typeScale.monoMeta, tabularNums, styles.timestamp, { color: tokens.ink4 }]}>
          {formatClock(row.created_at)}
        </Text>
        <Text style={[typeScale.section, { color }]}>{roleLabel(row.role)}</Text>
      </View>
      {multiline ? (
        <View style={styles.entryBody}>
          <SystemOutput content={row.content} />
        </View>
      ) : row.role === "system" ? (
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
  entryBody: { marginTop: space.space8, marginLeft: TIMESTAMP_COL_WIDTH + space.space8 },
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
