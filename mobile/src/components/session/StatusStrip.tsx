// Hearth core rule 8 (session titles are task titles; "repo · branch · model live in the
// mono meta line") + rule 7 (context as % in glanceable chrome — the raw token pair only
// belongs inside TelemetrySheet's detail view, which this still opens on tap). One JetBrains
// Mono row directly under SessionHeader's title: cwd/worktree · model on the left, cost ·
// ctx% (+ weekly quota, when the session is on a metered subscription plan) on the right.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { RemoteInput } from "../../lib/ws";
import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatCwd, tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { ContextGauge } from "../ds/ContextGauge";
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
  const { isCompact } = useBreakpoint();
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
          hitSlop={{ top: 10, bottom: 10 }}
        >
          <Text style={[typeScale.monoMeta, tabularNums, styles.left, { color: tokens.ink3 }]} numberOfLines={1}>
            {left}
          </Text>
          <View style={styles.right}>
            {props.reconnecting ? <Text style={[typeScale.monoMeta, { color: tokens.warn }]} numberOfLines={1}>{"reconnecting… · "}</Text> : null}
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]} numberOfLines={1}>{formatCost(props.costUsd)}</Text>
            {ctxPct != null ? (
              isCompact ? (
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>{` · ${ctxPct}% ctx`}</Text>
              ) : (
                <>
                  <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{" · "}</Text>
                  <View style={styles.gauge}>
                    <ContextGauge used={props.contextTokens} total={props.contextLimit ?? 0} />
                  </View>
                </>
              )
            ) : null}
            {props.weekly ? <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>{` · +${props.weekly.deltaPct.toFixed(1)}% wk`}</Text> : null}
          </View>
        </Pressable>
      </Animated.View>
      <TelemetrySheet {...props} visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

const styles = StyleSheet.create({
  // Visually a single mono meta line — a 44px band here reads as random dead space between the
  // title and the workflow pill. Keep the row tight and restore the 44px touch target via
  // hitSlop on the Pressable instead.
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 24, paddingVertical: 0 },
  left: { flex: 1, flexShrink: 1, minWidth: 0 },
  right: { flexDirection: "row", alignItems: "center", flexShrink: 0 },
  // Wide layouts have room for the real gauge (track + token pair) instead of a bare "NN% ctx".
  gauge: { width: 180 },
});
