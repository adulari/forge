// DESIGN_SYSTEM.md §6 `TaskRow`: glyph ring — pending = hollow circle ink3, in_progress =
// half-filled accent (rotating 2s while busy), done = filled success + strikethrough dim
// title. DESIGN_ELEVATION.md Move 2 (de-box): a hairline-separated row, not a Card — the
// glyph ring itself is the only "container-ish" affordance, no per-row box/fill.
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withRepeat, withTiming } from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { easings } from "../../theme/motion";
import { rowHeight, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import type { SnapshotTask } from "../../lib/ws";

const GLYPH_SIZE = 18;
const GLYPH_BORDER_WIDTH = 2;
const SPIN_MS = 2000;

export interface TaskRowProps {
  task: SnapshotTask;
  /** Session-level `busy` (Snapshot.busy) — gates the in_progress glyph's spin ("live" heat). */
  busy?: boolean;
  showSeparator?: boolean;
}

/** in_progress glyph spins 2s/rev while the session is busy; static otherwise (or reduced motion). */
function useTaskGlyphSpin(active: boolean) {
  const reduced = useReducedMotion();
  const spin = useSharedValue(0);

  React.useEffect(() => {
    if (!active || reduced) {
      spin.value = 0;
      return;
    }
    spin.value = withRepeat(withTiming(360, { duration: SPIN_MS, easing: easings.linear }), -1, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, reduced]);

  return useAnimatedStyle(() => ({ transform: [{ rotate: `${spin.value}deg` }] }));
}

function TaskGlyph({ status, busy }: { status: SnapshotTask["status"]; busy: boolean }) {
  const tokens = useTokens();
  const spinStyle = useTaskGlyphSpin(status === "in_progress" && busy);

  if (status === "done") {
    return (
      <View
        style={[
          styles.ring,
          { borderColor: tokens.success, backgroundColor: tokens.success },
        ]}
      />
    );
  }

  if (status === "in_progress") {
    return (
      <Animated.View style={[styles.ring, { borderColor: tokens.accent }, spinStyle]}>
        <View style={[styles.halfFill, { backgroundColor: tokens.accent }]} />
      </Animated.View>
    );
  }

  return <View style={[styles.ring, { borderColor: tokens.ink3 }]} />;
}

function TaskRowBase({ task, busy = false, showSeparator = true }: TaskRowProps) {
  const tokens = useTokens();
  const done = task.status === "done";
  const statusLabel = task.status === "in_progress" ? "in progress" : task.status;

  return (
    <View>
      <View
        style={styles.row}
        accessibilityRole="text"
        accessibilityLabel={`${task.title}, ${statusLabel}`}
      >
        <View style={styles.slot}>
          <TaskGlyph status={task.status} busy={busy} />
        </View>
        <Text
          style={[
            typeScale.body,
            styles.title,
            done
              ? { color: tokens.ink3, textDecorationLine: "line-through" }
              : { color: tokens.ink },
          ]}
          numberOfLines={2}
        >
          {task.title}
        </Text>
      </View>
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.border }]} /> : null}
    </View>
  );
}

export const TaskRow = React.memo(TaskRowBase);

const styles = StyleSheet.create({
  row: {
    minHeight: rowHeight.list,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space16,
    gap: space.space12,
  },
  slot: { alignItems: "center", justifyContent: "center" },
  title: { flex: 1 },
  ring: {
    width: GLYPH_SIZE,
    height: GLYPH_SIZE,
    borderRadius: GLYPH_SIZE / 2,
    borderWidth: GLYPH_BORDER_WIDTH,
    overflow: "hidden",
  },
  halfFill: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    height: "50%",
  },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
});
