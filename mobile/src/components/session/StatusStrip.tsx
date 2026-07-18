// Hearth core rule 8 (session titles are task titles; "repo · branch · model live in the
// mono meta line") + rule 7 (context as % in glanceable chrome — the raw token pair only
// belongs inside TelemetrySheet's detail view, which this still opens on tap). One JetBrains
// Mono row directly under SessionHeader's title: cwd/worktree · model on the left, cost ·
// ctx% (+ weekly quota, when the session is on a metered subscription plan) on the right.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";
import { LinearGradient } from "expo-linear-gradient";

import type { SessionTransportInfo, StripCondition } from "../../lib/anywhere/types";
import type { RemoteInput } from "../../lib/ws";
import { useEmberdot, useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type ColorTokens, type StatusDotState } from "../../theme/tokens";
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
   * of the old full-width Banner (which read as out-of-place chrome). Unrelated to `strip`
   * below: when `strip` is unset this keeps rendering exactly as it always has, inline in
   * the right-hand cost/ctx cluster — Forge Anywhere's 7-variant status line is a distinct,
   * additive row that only appears once a real relay-hosted session supplies `strip`. */
  reconnecting?: boolean;
  /** Forge Anywhere — when set, the left mono line grows a `host · transport` prefix ahead
   * of `cwd · model` (design comp "AW Session Transport", line 548). Absent for ordinary
   * direct sessions, which keep today's `cwd · model` line unchanged. */
  transport?: SessionTransportInfo;
  /** Forge Anywhere — one of the design's 7 session status-strip conditions, rendered as its
   * own row directly under the meta line (mobile.dc.html lines 552-591). */
  strip?: StripCondition;
  /** Fired when a strip condition's trailing action (e.g. "Switch to Direct", "Billing") is
   * pressed. The caller owns navigation/side-effects — this component only reports intent. */
  onStripAction?: (kind: string) => void;
}

export function StatusStrip(props: StatusStripProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const { isCompact } = useBreakpoint();
  const [visible, setVisible] = useState(false);
  const left = props.transport
    ? `${props.transport.hostName} · ${props.transport.transport} · ${formatCwd(props.cwd)} · ${props.model}`
    : `${formatCwd(props.cwd)} · ${props.model}`;
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
      {props.strip ? (
        <StripConditionRow
          strip={props.strip}
          hostName={props.transport?.hostName ?? "the host"}
          onAction={props.onStripAction}
        />
      ) : null}
      <TelemetrySheet {...props} visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

// ---------------------------------------------------------------------------
// Forge Anywhere — session status-strip condition row (7 variants, design comp
// "AW Session Transport" lines 552-591). One line shows at a time, directly under
// the meta row above.
// ---------------------------------------------------------------------------

interface ConditionVisual {
  dotColor: string;
  text: string;
  textColor: string;
  action?: { kind: string; label: string };
}

function conditionVisual(
  strip: Exclude<StripCondition, { kind: "capsule-upload" }>,
  hostName: string,
  tokens: ColorTokens,
): ConditionVisual {
  switch (strip.kind) {
    case "reconnecting-via-relay":
      return {
        dotColor: tokens.warn,
        // Matches the comp's literal text color (#FFD9A8 in dark) — that hex IS `warnBgInk`.
        text: `Reconnecting to ${hostName} via relay… retry in ${strip.retryInSec}s`,
        textColor: tokens.warnBgInk,
      };
    case "host-asleep-input-queued":
      return {
        dotColor: tokens.ink4,
        text: `${hostName} is asleep — your input queues until it wakes`,
        textColor: tokens.ink2,
      };
    case "relay-unreachable-direct-paired":
      return {
        dotColor: tokens.warn,
        text: `Relay unreachable · ${hostName} paired for Direct on this network`,
        textColor: tokens.ink2,
        action: { kind: "switch-direct", label: "Switch to Direct" },
      };
    case "relay-unreachable-not-paired":
      return {
        dotColor: tokens.warn,
        text: `Relay unreachable · ${hostName} seen on LAN, not paired for Direct`,
        textColor: tokens.ink2,
        action: { kind: "setup-direct", label: "Set up Direct connection" },
      };
    case "read-only-controlling-elsewhere":
      // The type carries no controlling-device name (unlike the comp's hardcoded
      // "MacBook Pro") — kept generic rather than fabricating one.
      return {
        dotColor: tokens.ink4,
        text: "Read-only — controlling elsewhere",
        textColor: tokens.ink2,
        action: { kind: "take-control", label: "Take control" },
      };
    case "plan-read-only":
      return {
        dotColor: tokens.danger,
        text: "Plan is read-only — you can watch, not steer",
        textColor: tokens.ink2,
        action: { kind: "billing", label: "Billing" },
      };
    default: {
      const _exhaustive: never = strip;
      return _exhaustive;
    }
  }
}

function StripConditionRow({
  strip,
  hostName,
  onAction,
}: {
  strip: StripCondition;
  hostName: string;
  onAction?: (kind: string) => void;
}) {
  const tokens = useTokens();
  // Only the "reconnecting" variant pulses in the comp — every other dot is static.
  const { dotStyle } = useEmberdot(strip.kind === "reconnecting-via-relay" ? "busy" : "idle");

  if (strip.kind === "capsule-upload") {
    const pct = Math.max(0, Math.min(100, strip.progressPct));
    return (
      <View style={styles.conditionRow} accessibilityRole="progressbar" accessibilityValue={{ min: 0, max: 100, now: Math.round(pct) }}>
        <Text style={[typeScale.monoMeta, tabularNums, styles.conditionText, { color: tokens.ink3 }]} numberOfLines={1}>
          {`capsule upload · ${Math.round(pct)}% · ${strip.mbPerSec.toFixed(1)} MB/s`}
        </Text>
        <View style={[styles.progressTrack, { backgroundColor: tokens.border }]}>
          <LinearGradient
            colors={[tokens.heatEdgeFrom, tokens.heatEdgeTo]}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            style={[styles.progressFill, { width: `${pct}%` }]}
          />
        </View>
      </View>
    );
  }

  const visual = conditionVisual(strip, hostName, tokens);
  return (
    <View style={styles.conditionRow} accessibilityRole="text" accessibilityLabel={visual.text}>
      <Animated.View style={[styles.conditionDot, { backgroundColor: visual.dotColor }, dotStyle]} />
      <Text style={[typeScale.sub, styles.conditionText, { color: visual.textColor }]} numberOfLines={1}>
        {visual.text}
      </Text>
      {visual.action ? (
        <Pressable
          onPress={() => onAction?.(visual.action!.kind)}
          accessibilityRole="button"
          accessibilityLabel={visual.action.label}
          hitSlop={{ top: 12, bottom: 12, left: 8, right: 8 }}
        >
          <Text style={[typeScale.meta, { color: tokens.accent, fontWeight: "600" }]} numberOfLines={1}>
            {visual.action.label}
          </Text>
        </Pressable>
      ) : null}
    </View>
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
  conditionRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 24, paddingVertical: 2 },
  conditionDot: { width: 7, height: 7, borderRadius: 3.5, flexShrink: 0 },
  conditionText: { flex: 1, minWidth: 0 },
  progressTrack: { width: 110, height: 3, borderRadius: 2, overflow: "hidden", flexShrink: 0 },
  progressFill: { height: "100%", borderRadius: 2 },
});
