// Workflow run progress header: run-state dot, name, done/total agents, summed cost, a
// client-side elapsed timer, and a thin progress bar. Stop is the only run control the wire
// supports (interrupt) — there is no pause verb, so no pause button is drawn.
import { ArrowLeft, Square } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import { formatDuration } from "./format";

function RunDot({ active, ok }: { active: boolean; ok: boolean | null }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(active ? "busy" : "idle");
  const color = active ? tokens.accent : ok === false ? tokens.danger : ok === true ? tokens.success : tokens.ink3;
  return (
    <View style={styles.runDotWrap}>
      {active ? <View style={[styles.runDotGlow, { backgroundColor: tokens.dotGlow }]} /> : null}
      <Animated.View style={[styles.runDot, { backgroundColor: color }, dotStyle]} />
    </View>
  );
}

export function ProgressHeader({
  title,
  active,
  finishedOk,
  doneCount,
  totalCount,
  cost,
  elapsedSeconds,
  onBack,
  onStop,
}: {
  title: string;
  active: boolean;
  finishedOk: boolean | null;
  doneCount: number;
  totalCount: number;
  cost: number;
  elapsedSeconds: number;
  onBack: () => void;
  onStop?: () => void;
}) {
  const tokens = useTokens();
  const pct = totalCount > 0 ? Math.round((doneCount / totalCount) * 100) : 0;

  return (
    <View style={styles.wrap}>
      <View style={styles.titleRow}>
        <Pressable
          onPress={onBack}
          accessibilityRole="button"
          accessibilityLabel="Back to workflows"
          hitSlop={6}
          style={styles.back}
        >
          <ArrowLeft size={20} strokeWidth={2} color={tokens.ink2} />
        </Pressable>
        <RunDot active={active} ok={finishedOk} />
        <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          {title}
        </Text>
        {active && onStop ? (
          <Pressable
            onPress={onStop}
            accessibilityRole="button"
            accessibilityLabel="Stop workflow"
            style={[styles.stop, { borderColor: tokens.borderStrong }]}
          >
            <Square size={12} strokeWidth={0} fill={tokens.danger} color={tokens.danger} />
          </Pressable>
        ) : null}
      </View>

      <View style={styles.metaRow}>
        <Text style={[typeScale.monoMeta, tabularNums, styles.metaLeft, { color: tokens.ink3 }]} numberOfLines={1}>
          {`${doneCount}/${totalCount} agents done`}
        </Text>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
          <Text style={{ color: tokens.success }}>{formatCost(cost)}</Text>
          {` · ${formatDuration(elapsedSeconds)}`}
        </Text>
      </View>

      <View style={styles.progressRow}>
        <View style={[styles.track, { backgroundColor: tokens.border }]}>
          <View style={[styles.fill, { width: `${pct}%`, backgroundColor: tokens.accent }]} />
        </View>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{`${pct}%`}</Text>
      </View>
    </View>
  );
}

const META_INDENT = 34;

const styles = StyleSheet.create({
  wrap: { gap: space.space8 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  back: { width: 28, height: 44, marginLeft: -6, alignItems: "center", justifyContent: "center" },
  title: { flex: 1, fontSize: 19 },
  stop: {
    width: 34,
    height: 34,
    borderRadius: radii.radius8,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  metaRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginLeft: META_INDENT },
  metaLeft: { flex: 1 },
  progressRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginLeft: META_INDENT },
  track: { flex: 1, height: 3, borderRadius: 2, overflow: "hidden" },
  fill: { height: "100%", borderRadius: 2 },
  runDotWrap: { width: 8, height: 8, alignItems: "center", justifyContent: "center" },
  runDot: { width: 8, height: 8, borderRadius: 4 },
  runDotGlow: { position: "absolute", width: 16, height: 16, borderRadius: 8 },
});
