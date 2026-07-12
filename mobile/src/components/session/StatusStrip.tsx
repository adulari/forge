import React, { useState } from "react";
import { ChevronRight } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { RemoteInput } from "../../lib/ws";
import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { ContextGauge } from "../ds/ContextGauge";
import { CostMetric } from "../ds/CostMetric";
import { StatusDot } from "../ds/StatusDot";
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
}

export function StatusStrip(props: StatusStripProps) {
  const tokens = useTokens();
  const strike = useStrike();
  const [visible, setVisible] = useState(false);
  const tierModel = props.tier ? `${props.tier} · ${props.model}` : props.model;

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
          <StatusDot state={props.state} />
          <Text style={[typeScale.meta, styles.tierModel, { color: tokens.ink2 }]} numberOfLines={1}>
            {tierModel}
          </Text>
          {props.weekly ? (
            <Text
              style={[typeScale.meta, tabularNums, { color: tokens.success }]}
              numberOfLines={1}
              accessibilityLabel={`weekly quota +${props.weekly.deltaPct.toFixed(1)}% this session`}
            >
              +{props.weekly.deltaPct.toFixed(1)}% wk
            </Text>
          ) : <CostMetric valueUsd={props.costUsd} />}
          {props.contextLimit != null ? (
            <View style={styles.gauge}>
              <ContextGauge used={props.contextTokens} total={props.contextLimit} compact />
            </View>
          ) : null}
          <ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </Animated.View>
      <TelemetrySheet {...props} visible={visible} onClose={() => setVisible(false)} />
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 52, paddingVertical: space.space8 },
  tierModel: { flex: 1, flexShrink: 1, minWidth: 0 },
  gauge: { width: 96, flexShrink: 0 },
});
