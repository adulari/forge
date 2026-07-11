// Session shell status strip (T3.1, DESIGN_SYSTEM.md §6/§1.4): StatusDot(idle|busy|
// waiting|done), tier·model, temper Chip, CostMetric, ContextGauge — sits under the header.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { ContextGauge } from "../ds/ContextGauge";
import { CostMetric } from "../ds/CostMetric";
import { EffortPicker } from "./EffortPicker";
import { StatusDot } from "../ds/StatusDot";

export interface StatusStripProps {
  state: StatusDotState;
  tier: string | null;
  model: string;
  temper: string;
  effort?: string | null;
  send: (input: RemoteInput) => void;
  costUsd: number;
  contextTokens: number;
  contextLimit: number | null;
}

export function StatusStrip({
  state,
  tier,
  model,
  temper,
  effort,
  send,
  costUsd,
  contextTokens,
  contextLimit,
}: StatusStripProps) {
  const tokens = useTokens();
  const tierModel = tier ? `${tier} · ${model}` : model;

  return (
    <View style={styles.row}>
      <StatusDot state={state} />
      <Text style={[typeScale.meta, styles.tierModel, { color: tokens.ink2 }]} numberOfLines={1}>
        {tierModel}
      </Text>
      <Chip label={temper} />
      <EffortPicker effort={effort} send={send} />
      <CostMetric valueUsd={costUsd} />
      {contextLimit != null ? (
        <View style={styles.gauge}>
          <ContextGauge used={contextTokens} total={contextLimit} />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  tierModel: { flexShrink: 1 },
  gauge: { flex: 1, minWidth: 80 },
});
