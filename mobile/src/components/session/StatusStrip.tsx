// Hearth core rule 8 (session titles are task titles; "repo · branch · model live in the
// mono meta line") + rule 7 (context as % in glanceable chrome — the raw token pair only
// belongs inside TelemetrySheet's detail view, which this still opens on tap). One JetBrains
// Mono row directly under SessionHeader's title: cwd/worktree · model on the left, cost ·
// ctx% (+ weekly quota, when the session is on a metered subscription plan) on the right.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import Animated from "react-native-reanimated";

import type { RemoteInput } from "../../lib/ws";
import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatCwd, tabularNums, type as typeScale } from "../../theme/typography";
import { TelemetrySheet } from "./TelemetrySheet";

export interface StatusStripProps {
  state: StatusDotState;
  tier: string | null;
  model: string;
  temper: string;
  effort?: string | null;
  send: (input: RemoteInput) => boolean;
  costUsd: number;
  contextTokens: number;
  contextLimit: number | null;
  weekly?: { provider: string; deltaPct: number } | null;
  cwd: string;
  worktree: string | null;
  /** Soft connection blip — renders as a small warm mono note inside the meta line instead
   * of the old full-width Banner (which read as out-of-place chrome). */
  reconnecting?: boolean;
}

export function StatusStrip(props: StatusStripProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [visible, setVisible] = useState(false);
  const left = `${formatCwd(props.cwd)} · ${props.model}`;
  const ctxPct =
    props.contextLimit != null && props.contextLimit > 0
      ? Math.min(100, Math.round((props.contextTokens / props.contextLimit) * 100))
      : null;

  return (
    <>
      <Animated.View style={strike.style}>
        <Pressable
          onPress={() => setVisible(true)}
          onPressIn={strike.onPressIn}
          onPressOut={strike.onPressOut}
          accessibilityRole="button"
          accessibilityLabel="Open session telemetry"
          style={styles.row}
        >
          <Text style={[typeScale.monoMeta, tabularNums, styles.left, { color: tokens.ink3 }]} numberOfLines={1}>
            {left}
          </Text>
          <Text style={[typeScale.monoMeta, tabularNums, styles.right, { color: tokens.ink3 }]} numberOfLines={1}>
            {props.reconnecting ? <Text style={{ color: tokens.warn }}>{"reconnecting… · "}</Text> : null}
            <Text style={{ color: tokens.success }}>{formatCost(props.costUsd)}</Text>
            {ctxPct != null ? ` · ${ctxPct}% ctx` : ""}
            {props.weekly ? ` · +${props.weekly.deltaPct.toFixed(1)}% wk` : ""}
          </Text>
        </Pressable>
      </Animated.View>
      <TelemetrySheet {...props} visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44, paddingVertical: space.space4 },
  left: { flex: 1, flexShrink: 1, minWidth: 0 },
  right: { flexShrink: 0 },
});
