// DESIGN_ELEVATION.md Move 2 (de-box) SessionCard (fleet): hairline-separated row, no
// per-row border/box/fill — StatusDot + title + NEEDS-YOU/worktree badges on the title
// line; metadata line cwd tail (mono, head-ellipsized) · model · relative time; quiet
// gauge+cost line. Live (busy/waiting) rows carry a HeatEdge bleeding to the row's left
// edge; waiting rows additionally get a `selection` wash. Swipe (native,
// react-native-gesture-handler) / long-press / trailing `…` all open the SAME
// archive/merge/discard actions — merge 409s and discard warnings never render as a generic
// toast (FEATURES.md §1.1), they get their own result sheet.
import { router } from "expo-router";
import { Archive, Ellipsis, GitMerge, Trash2 } from "lucide-react-native";
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
import { haptics } from "../../lib/haptics";
import { useArchiveSession, useDiscardSession, useMergeSession } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { springs, useForgeline, useSettle } from "../../theme/motion";
import { space, type StatusDotState } from "../../theme/tokens";
import { formatCwd, monoFamily, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { ContextGauge } from "../ds/ContextGauge";
import { CostMetric } from "../ds/CostMetric";
import { HeatEdge } from "../ds/HeatEdge";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { RelativeTime } from "../ds/RelativeTime";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";

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

  const archive = useArchiveSession();
  const merge = useMergeSession();
  const discard = useDiscardSession();

  const [actionsVisible, setActionsVisible] = useState(false);
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

  const openActions = useCallback(() => {
    closeSwipe();
    setActionsVisible(true);
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
              <Pressable
                ref={rowRef}
                onPress={goToSession}
                onLongPress={openActions}
                accessibilityRole={Platform.OS === "web" ? undefined : "button"}
                accessibilityLabel={`${title}, ${state}, ${row.cwd}`}
                accessibilityState={{ selected }}
              >
                <View
                  style={[styles.rowBg, { backgroundColor: row.waiting || selected ? tokens.selection : tokens.bg1 }]}
                >
                  <HeatEdge state={row.waiting ? "waiting" : row.busy ? "busy" : false} />
                  <View style={styles.inner}>
                    <View style={styles.row1}>
                      <StatusDot state={state} />
                      <Text style={[typeScale.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
                        {title}
                      </Text>
                      {hasWorktree ? <Badge label="worktree" tone="outline" /> : null}
                      {row.waiting ? (
                        <RelativeTime timestampMs={row.last_activity * 1000} />
                      ) : (
                        <View style={styles.metricStack}>
                          <CostMetric valueUsd={row.cost_usd} />
                          <RelativeTime timestampMs={row.last_activity * 1000} style={typeScale.monoMeta} />
                        </View>
                      )}
                      <IconButton
                        icon={<Ellipsis size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink3} />}
                        onPress={openActions}
                        accessibilityLabel="Session actions"
                      />
                    </View>

                    {row.waiting ? (
                      <Text
                        style={[typeScale.codeSmall, { color: tokens.ink2, fontFamily: monoFamily.regular }]}
                        numberOfLines={1}
                      >
                        needs a decision
                      </Text>
                    ) : (
                      <View style={styles.row2}>
                        <Text
                          style={[
                            typeScale.codeSmall,
                            styles.cwd,
                            { color: tokens.ink3, fontFamily: monoFamily.regular },
                          ]}
                          numberOfLines={1}
                          ellipsizeMode="head"
                          accessibilityLabel={`path: ${row.cwd}`}
                        >
                          {cwdLabel} · {row.model}
                        </Text>
                      </View>
                    )}

                    {!row.waiting && row.context_limit != null ? (
                      <View style={styles.row3}>
                        <ContextGauge used={row.context_tokens} total={row.context_limit} />
                      </View>
                    ) : null}
                  </View>
                </View>
              </Pressable>
            </Animated.View>
          </GestureDetector>

          <View style={[styles.separator, { backgroundColor: tokens.border }]} />
        </View>
      </Animated.View>

      <Sheet visible={actionsVisible} onClose={() => setActionsVisible(false)} accessibilityLabel="Session actions">
        <View style={styles.sheetBody}>
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
    a.context_tokens === b.context_tokens &&
    a.context_limit === b.context_limit &&
    a.model === b.model &&
    a.last_activity === b.last_activity
  );
});

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  actionsRow: { position: "absolute", top: 0, bottom: 0, right: 0, flexDirection: "row" },
  actionButton: { height: "100%", width: ACTION_WIDTH, borderRadius: 0, borderWidth: 0 },
  // De-boxed row (DESIGN_ELEVATION.md Move 2): no border/fill/radius — the row's own
  // hairline separator (below) is the only division between sessions.
  rowBg: { position: "relative" },
  heatEdgeWrap: { position: "absolute", left: 0, top: 0, bottom: 0 },
  inner: {
    minHeight: 72,
    justifyContent: "center",
    paddingHorizontal: space.space16,
    paddingVertical: space.space16,
    gap: space.space16,
  },
  row1: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  metricStack: { alignItems: "flex-end", gap: 1 },
  row2: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  cwd: { flex: 1 },
  row3: { width: "100%" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
  sheetBody: { paddingHorizontal: space.space4, paddingBottom: space.space16, gap: space.space4 },
  fileRow: { paddingHorizontal: space.space16 },
});
