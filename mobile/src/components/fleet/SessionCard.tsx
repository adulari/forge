// Machined SessionCard (fleet quiet row): a bordered Card — StatusDot + title + a single
// right-aligned mono metric on the header line, mono meta line below (host · transport ·
// cwd tail · model). Matches the "quiet tool row" card treatment used by the needs-you
// DecisionCard sibling (same Card primitive, no per-row heat/glow — Machined retired the
// thermal identity entirely). Swipe (native, react-native-gesture-handler) /
// long-press / trailing `…` all open the SAME archive/merge/discard actions — merge 409s
// and discard warnings never render as a generic toast (FEATURES.md §1.1), they get their
// own result sheet.
import { router } from "expo-router";
import { Archive, GitMerge, MoreHorizontal, Settings2, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSpring,
} from "react-native-reanimated";

import { ApiError, type MergeDirtyConflictResponse, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { haptics } from "../../lib/haptics";
import { useArchiveSession, useDiscardSession, useMergeSession } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { springs, useForgeline, useSettle } from "../../theme/motion";
import { cardPadding, space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatCwd, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Card } from "../ds/Card";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { RelativeTime } from "../ds/RelativeTime";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { SessionLifecycleSheet } from "../session/SessionLifecycleSheet";

// Hearth "ONE right-aligned mono metric" (HANDOFF Fleet rows): cost while forging,
// relative time while cool, elapsed while waiting — never stacked, never with CostMetric's
// fixed success-green (the prototype renders this metric ink2/ink3/ink4, not success).
function SessionMetric({ row, state }: { row: SessionRow; state: StatusDotState }) {
  const tokens = useTokens();
  if (state === "waiting") {
    return (
      <RelativeTime
        timestampMs={row.last_activity * 1000}
        style={{ ...tabularNums, fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15, color: tokens.ink3 }}
      />
    );
  }
  if (state === "busy") {
    return (
      <Text style={[styles.metric, tabularNums, { fontFamily: monoFamily.regular, color: tokens.ink2 }]} numberOfLines={1}>
        {formatCost(row.cost_usd)}
      </Text>
    );
  }
  return (
    <RelativeTime
      timestampMs={row.last_activity * 1000}
      style={{ ...tabularNums, fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: 14, color: tokens.ink4 }}
    />
  );
}

export interface SessionCardProps {
  row: SessionRow;
  /** Position in the (server-sorted) list — drives Forgeline stagger, nothing else. */
  index: number;
  /** Highlights the session currently rendered in an expanded detail pane. */
  selected?: boolean;
}

const ACTION_WIDTH = 64;
const ICON_SIZE = 20;
const ICON_STROKE = 1.75;

function SessionCardBase({ row, index, selected = false }: SessionCardProps) {
  const tokens = useTokens();
  const toast = useToast();
  const reduced = useReducedMotion();
  const entranceStyle = useForgeline(index);
  // Forge Anywhere: host + transport prefix on the meta line (design mobile.dc.html
  // "AW Fleet" row meta, e.g. "MacBook Pro · direct · forge/relay · gpt-5.5"). Every real
  // session today is served by the active server over direct transport — a relay-backed
  // session will carry transport "anywhere" once Forge Anywhere sessions land here.
  const { servers, activeServerId } = useAuth();
  const hostLabel = servers.find((s) => s.id === activeServerId)?.name ?? "this server";

  const archive = useArchiveSession();
  const merge = useMergeSession();
  const discard = useDiscardSession();

  const [actionsVisible, setActionsVisible] = useState(false);
  const [lifecycleVisible, setLifecycleVisible] = useState(false);
  const [archiveConfirmVisible, setArchiveConfirmVisible] = useState(false);
  const [discardConfirmVisible, setDiscardConfirmVisible] = useState(false);
  const [mergeResult, setMergeResult] = useState<MergeDirtyConflictResponse | null>(null);
  const [discardWarnings, setDiscardWarnings] = useState<string[] | null>(null);
  const rowRef = useRef<React.ComponentRef<typeof Pressable>>(null);

  const hasWorktree = !!row.worktree;
  const title = row.title || `session ${row.id.slice(0, 8)}`;
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const actionCount = hasWorktree ? 3 : 1;
  const actionsWidth = ACTION_WIDTH * actionCount;

  const translateX = useSharedValue(0);
  const settleStyle = useSettle(state);
  const previousState = useRef(state);
  useEffect(() => {
    if (state === "waiting" && previousState.current !== "waiting") haptics.select();
    previousState.current = state;
  }, [state]);
  const cwdLabel = formatCwd(row.cwd);

  const closeSwipe = useCallback(() => {
    translateX.value = reduced ? 0 : withSpring(0, springs.press);
  }, [reduced, translateX]);

  // Archive/merge/discard all stop a driver — there is none for a terminal-local (`read_only`)
  // session, so the daemon already 404s these; refuse in the UI too instead of surfacing that as
  // a confusing error toast.
  const openActions = useCallback(() => {
    if (row.read_only) {
      toast.show("read-only — running in a terminal, not this daemon", { tone: "neutral" });
      return;
    }
    closeSwipe();
    setActionsVisible(true);
  }, [closeSwipe, row.read_only, toast]);

  const openLifecycle = useCallback(() => {
    closeSwipe();
    setActionsVisible(false);
    setLifecycleVisible(true);
  }, [closeSwipe]);

  const runArchive = useCallback(() => {
    closeSwipe();
    setActionsVisible(false);
    setArchiveConfirmVisible(true);
  }, [closeSwipe]);

  const confirmArchive = useCallback(() => {
    setArchiveConfirmVisible(false);
    archive.mutate(row.id, {
      onError: (err) => {
        haptics.mergeConflict();
        toast.show(err instanceof ApiError ? err.message : "archive failed", { tone: "danger" });
      },
    });
  }, [archive, row.id, toast]);

  const runMerge = useCallback(() => {
    closeSwipe();
    setActionsVisible(false);
    merge.mutate(row.id, {
      onSuccess: (res) => {
        haptics.pairSuccess();
        toast.show(`merged branch ${res.branch}`, { tone: "success" });
      },
      onError: (err) => {
        haptics.mergeConflict();
        if (err instanceof ApiError && err.status === 409) {
          setMergeResult((err.body as MergeDirtyConflictResponse | undefined) ?? { error: err.message });
        } else {
          toast.show(err instanceof ApiError ? err.message : "merge failed", { tone: "danger" });
        }
      },
    });
  }, [closeSwipe, merge, row.id, toast]);

  const runDiscard = useCallback(() => {
    closeSwipe();
    setActionsVisible(false);
    setDiscardConfirmVisible(true);
  }, [closeSwipe]);

  const goToSession = useCallback(() => {
    router.push(`/session/${row.id}`);
  }, [row.id]);

  // The row's trailing `…` IconButton is a real nested <button>, so on react-native-web the
  // row itself can't also be an actual <button> (accessibilityRole="button" would render one) —
  // that's an invalid button-in-button and breaks hydration. Keep the row a plain focusable
  // <div> on web and replicate Space-to-activate manually; Enter already works unconditionally
  // via RNW's press responder. Native (iOS/Android) keeps accessibilityRole="button" as-is.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = rowRef.current as unknown as HTMLElement | null;
    if (!node) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === " " || e.key === "Spacebar") {
        e.preventDefault();
        goToSession();
      }
    };
    node.addEventListener("keydown", onKeyDown);
    return () => node.removeEventListener("keydown", onKeyDown);
  }, [goToSession]);

  const confirmDiscard = useCallback(() => {
    setDiscardConfirmVisible(false);
    discard.mutate(row.id, {
      onSuccess: (res) => {
        if (res.warnings.length > 0) setDiscardWarnings(res.warnings);
      },
      onError: (err) => {
        haptics.mergeConflict();
        toast.show(err instanceof ApiError ? err.message : "discard failed", { tone: "danger" });
      },
    });
  }, [discard, row.id, toast]);

  const pan = Gesture.Pan()
    .enabled(Platform.OS !== "web")
    .activeOffsetX([-10, 10])
    .onUpdate((e) => {
      translateX.value = Math.max(-actionsWidth, Math.min(0, e.translationX));
    })
    .onEnd((e) => {
      const pastHalf = translateX.value < -actionsWidth / 2;
      const target = pastHalf || e.velocityX < -500 ? -actionsWidth : 0;
      translateX.value = reduced ? target : withSpring(target, springs.press);
    });

  const cardStyle = useAnimatedStyle(() => ({ transform: [{ translateX: translateX.value }] }));

  return (
    <>
      <Animated.View style={[entranceStyle, settleStyle]}>
        <View style={styles.wrap}>
          {Platform.OS !== "web" ? (
            <View style={[styles.actionsRow, { width: actionsWidth }]} pointerEvents="box-none">
              <IconButton
                icon={<Archive size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />}
                onPress={runArchive}
                accessibilityLabel="Archive session"
                style={[styles.actionButton, { backgroundColor: tokens.bg3 }]}
              />
              {hasWorktree ? (
                <IconButton
                  icon={<GitMerge size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.onAccent} />}
                  onPress={runMerge}
                  accessibilityLabel="Merge worktree"
                  style={[styles.actionButton, { backgroundColor: tokens.success }]}
                />
              ) : null}
              {hasWorktree ? (
                <IconButton
                  icon={<Trash2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.onAccent} />}
                  onPress={runDiscard}
                  accessibilityLabel="Discard worktree"
                  style={[styles.actionButton, { backgroundColor: tokens.danger }]}
                />
              ) : null}
            </View>
          ) : null}

          <GestureDetector gesture={pan}>
            <Animated.View style={cardStyle}>
              <Card
                padded={false}
                style={[
                  styles.card,
                  { backgroundColor: row.waiting || selected ? tokens.selection : tokens.bg2 },
                ]}
              >
                <Pressable
                  ref={rowRef}
                  onPress={goToSession}
                  onLongPress={openActions}
                  accessibilityRole={Platform.OS === "web" ? undefined : "button"}
                  accessibilityLabel={`${title}, ${state}, ${row.cwd}`}
                  accessibilityState={{ selected }}
                  style={styles.cardPressable}
                >
                  <View style={styles.row1}>
                    <StatusDot state={state} size={state === "idle" ? 7 : 8} />
                    <Text
                      style={[
                        styles.title,
                        {
                          fontSize: state === "idle" ? 13.5 : 14.5,
                          lineHeight: state === "idle" ? 19 : 20,
                          color: state === "idle" ? tokens.ink2 : tokens.ink,
                        },
                      ]}
                      numberOfLines={1}
                    >
                      {title}
                    </Text>
                    {row.read_only ? <Badge label="local" tone="outline" /> : null}
                    <SessionMetric row={row} state={state} />
                  </View>

                  <Text
                    style={[
                      styles.meta,
                      {
                        fontSize: state === "idle" ? 11 : 11.5,
                        color: state === "waiting" ? tokens.ink2 : state === "busy" ? tokens.ink3 : tokens.ink4,
                        fontFamily: monoFamily.regular,
                      },
                    ]}
                    numberOfLines={1}
                    ellipsizeMode={row.waiting ? "tail" : "head"}
                      accessibilityLabel={row.waiting ? undefined : `path: ${row.cwd}`}
                    >
                      {row.waiting ? "needs a decision" : `${hostLabel} · direct · ${cwdLabel} · ${row.model}`}
                    </Text>
                </Pressable>
                <IconButton
                  icon={
                    <MoreHorizontal
                      size={ICON_SIZE}
                      strokeWidth={ICON_STROKE}
                      color={tokens.ink3}
                    />
                  }
                  onPress={openActions}
                  accessibilityLabel={`Actions for ${title}`}
                  style={styles.moreButton}
                />
              </Card>
            </Animated.View>
          </GestureDetector>
        </View>
      </Animated.View>

      <Sheet visible={actionsVisible} onClose={() => setActionsVisible(false)} accessibilityLabel="Session actions">
        <View style={styles.sheetBody}>
          <ListRow
            title="Rename or archive session"
            leading={<Settings2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />}
            onPress={openLifecycle}
          />
          <ListRow
            title="Archive"
            leading={<Archive size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />}
            onPress={runArchive}
          />
          {hasWorktree ? (
            <ListRow
              title="Merge worktree"
              leading={<GitMerge size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />}
              onPress={runMerge}
            />
          ) : null}
          {hasWorktree ? (
            <ListRow
              title="Discard worktree"
              leading={<Trash2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.danger} />}
              onPress={runDiscard}
              showSeparator={false}
            />
          ) : null}
        </View>
      </Sheet>
      <SessionLifecycleSheet
        target={{
          id: row.id,
          title,
          cwd: row.cwd,
          archived: false,
          running: true,
        }}
        visible={lifecycleVisible}
        onClose={() => setLifecycleVisible(false)}
      />

      <ConfirmDialog
        visible={archiveConfirmVisible}
        title="Archive session"
        message={`Stop and hide "${title}" — history is kept.`}
        confirmLabel="Archive"
        cancelLabel="Cancel"
        onConfirm={confirmArchive}
        onCancel={() => setArchiveConfirmVisible(false)}
      />

      <ConfirmDialog
        visible={discardConfirmVisible}
        title={`Discard branch \`${row.worktree ?? title}\``}
        message="Unmerged work is lost."
        confirmLabel="Discard"
        cancelLabel="Cancel"
        destructive
        onConfirm={confirmDiscard}
        onCancel={() => setDiscardConfirmVisible(false)}
      />

      <Sheet
        visible={mergeResult != null}
        onClose={() => setMergeResult(null)}
        accessibilityLabel="Merge result"
      >
        <View style={styles.sheetBody}>
          <Text style={[typeScale.heading, { color: tokens.ink }]}>
            {mergeResult?.conflicts ? "Merge conflicts" : "Can't merge — uncommitted changes"}
          </Text>
          {mergeResult?.error ? (
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{mergeResult.error}</Text>
          ) : null}
          {(mergeResult?.dirty_files ?? mergeResult?.conflicts ?? []).map((f) => (
            <Text key={f} style={[typeScale.codeSmall, styles.fileRow, { color: tokens.ink2 }]} numberOfLines={1}>
              {f}
            </Text>
          ))}
        </View>
      </Sheet>

      <Sheet
        visible={discardWarnings != null}
        onClose={() => setDiscardWarnings(null)}
        accessibilityLabel="Discard warnings"
      >
        <View style={styles.sheetBody}>
          <Text style={[typeScale.heading, { color: tokens.ink }]}>Discarded — warnings</Text>
          {(discardWarnings ?? []).map((w) => (
            <Text key={w} style={[typeScale.sub, styles.fileRow, { color: tokens.warn }]}>
              {w}
            </Text>
          ))}
        </View>
      </Sheet>
    </>
  );
}

export const SessionCard = React.memo(SessionCardBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.selected === next.selected &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.worktree === b.worktree &&
    a.busy === b.busy &&
    a.waiting === b.waiting &&
    a.cost_usd === b.cost_usd &&
    a.model === b.model &&
    a.last_activity === b.last_activity
  );
});

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  // Aligned to the card's own inset/gap (space16 margin, space12 bottom gap) so the
  // revealed actions sit flush against the card's real edges, not the full-bleed row.
  actionsRow: { position: "absolute", top: 0, bottom: space.space12, right: space.space16, flexDirection: "row" },
  actionButton: { height: "100%", width: ACTION_WIDTH, borderRadius: 0, borderWidth: 0 },
  // Machined card row — same gap-separated bordered treatment as DecisionCard, so the
  // quiet rows and the needs-you card read as one consistent list.
  card: { marginHorizontal: space.space16, marginBottom: space.space12 },
  cardPressable: { paddingHorizontal: cardPadding.x, paddingVertical: cardPadding.y },
  moreButton: { position: "absolute", right: 0, bottom: 0 },
  row1: { flexDirection: "row", alignItems: "center", gap: 9 },
  title: { flex: 1, fontWeight: "600" },
  meta: { marginTop: 2, paddingRight: 36 },
  metric: { fontSize: 11, lineHeight: 15 },
  sheetBody: { paddingHorizontal: space.space4, paddingBottom: space.space16, gap: space.space4 },
  fileRow: { paddingHorizontal: space.space16 },
});
