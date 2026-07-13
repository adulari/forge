import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCost, formatTokenPair, type as typeScale } from "../../theme/typography";
import { ContextGauge } from "../ds/ContextGauge";
import { KeyValueRow } from "../ds/KeyValueRow";
import { Sheet } from "../ds/Sheet";
import { EFFORT_LEVELS, type EffortLevel } from "./EffortPicker";

export function TelemetrySheet({ visible, onClose, tier, model, temper, effort, send, costUsd, contextTokens, contextLimit, weekly }: {
  visible: boolean; onClose: () => void; tier: string | null; model: string; temper: string; effort?: string | null;
  send: (input: RemoteInput) => boolean; costUsd: number; contextTokens: number; contextLimit: number | null;
  weekly?: { provider: string; deltaPct: number } | null;
}) {
  const tokens = useTokens();
  const current = EFFORT_LEVELS.includes(effort as EffortLevel) ? effort : null;
  const select = (level: EffortLevel | null) => {
    if (send({ kind: "prompt", text: level ? `/effort ${level}` : "/effort" })) onClose();
  };
  const openTemperPicker = () => {
    if (send({ kind: "prompt", text: "/mode" })) onClose();
  };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Session telemetry" snapPoints={[0.8]}>
    <View style={styles.content}>
      <Text style={[typeScale.heading, { color: tokens.ink }]}>Session telemetry</Text>
      {contextLimit != null ? <><ContextGauge used={contextTokens} total={contextLimit} /><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{formatTokenPair(contextTokens, contextLimit)}</Text></> : null}
      {weekly ? <>
        <Text style={[typeScale.bodyBold, { color: tokens.success }]}>≈ +{weekly.deltaPct.toFixed(1)}% of weekly quota this session</Text>
        <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Approximate — may be off if other sessions or tools share this {weekly.provider} subscription.</Text>
      </> : <Text style={[typeScale.bodyBold, { color: tokens.success }]}>cost {formatCost(costUsd)}</Text>}
      <View style={styles.meshAction}><Pressable onPress={() => { if (send({ kind: "prompt", text: "/mesh" })) onClose(); }} accessibilityRole="button" style={[styles.meshButton, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}><Text style={[typeScale.bodyBold, { color: tokens.accent }]}>Explain mesh routing</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>See why Forge would choose this model</Text></Pressable></View>
      <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Operating mode</Text>
      <Pressable onPress={openTemperPicker} accessibilityRole="button" style={[styles.option, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}><Text style={[typeScale.bodyBold, { color: tokens.accent }]}>Change operating mode</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{temper}</Text></Pressable>
      <View style={[styles.rows, { borderTopColor: tokens.border }]}>
        <KeyValueRow label="Tier" value={tier ?? "—"} />
        <KeyValueRow label="Model" value={model} />
        <KeyValueRow label="Temper" value={temper} />
      </View>
      <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Reasoning effort</Text>
      <View style={styles.options}>
        {[null, ...EFFORT_LEVELS].map((level) => {
          const selected = level === current;
          const label = level ?? "default";
          return <Pressable key={label} onPress={() => select(level)} accessibilityRole="radio" accessibilityState={{ selected }} style={[styles.option, { backgroundColor: selected ? tokens.selection : tokens.bg2, borderColor: selected ? tokens.accent : tokens.border }]}>
            <Text style={[typeScale.bodyBold, { color: level === "whitehot" ? tokens.accent : tokens.ink }]}>{label}</Text>
            {selected ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>current</Text> : null}
          </Pressable>;
        })}
      </View>
    </View>
  </Sheet>;
}
const styles = StyleSheet.create({ content: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space8 }, meshAction: { paddingVertical: space.space4 }, meshButton: { minHeight: 44, justifyContent: "center", gap: 2, paddingHorizontal: space.space12, borderWidth: StyleSheet.hairlineWidth, borderRadius: 8 }, rows: { borderTopWidth: StyleSheet.hairlineWidth }, options: { gap: space.space8 }, option: { minHeight: 44, flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingHorizontal: space.space12, borderWidth: StyleSheet.hairlineWidth, borderRadius: 8 } });
