// Hearth session telemetry — de-boxed type-first sheet: hairline rows, mono metrics,
// one context gauge, accent only on actions. The raw token pair belongs HERE (rule 7:
// glanceable chrome shows %, the detail view shows `128.4k / 200k`).
import { ArrowLeft, ChevronRight } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { gaugeColor, space } from "../../theme/tokens";
import { formatCost, formatTokenPair, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Sheet } from "../ds/Sheet";
import { EFFORT_LEVELS, type EffortLevel } from "./EffortPicker";

export function TelemetrySheet({ visible, onClose, tier, model, temper, effort, send, costUsd, contextTokens, contextLimit, weekly }: { visible: boolean; onClose: () => void; tier: string | null; model: string; temper: string; effort?: string | null; send: (input: RemoteInput) => boolean; costUsd: number; contextTokens: number; contextLimit: number | null; weekly?: { provider: string; deltaPct: number } | null; }) {
  const tokens = useTokens();
  const [pane, setPane] = useState<"telemetry" | "effort">("telemetry");
  const current = EFFORT_LEVELS.includes(effort as EffortLevel) ? (effort as EffortLevel) : null;
  const contextPercent =
    contextLimit != null && contextLimit > 0 ? Math.min(100, (contextTokens / contextLimit) * 100) : null;
  const close = () => {
    setPane("telemetry");
    onClose();
  };
  const run = (command: string) => {
    if (send({ kind: "prompt", text: command })) close();
  };
  const selectEffort = (level: EffortLevel | null) => run(level ? `/effort ${level}` : "/effort");

  return (
    <Sheet visible={visible} onClose={close} accessibilityLabel={pane === "effort" ? "Reasoning effort" : "Session telemetry"} snapPoints={[0.82]}>
      <View style={styles.content}>
        {pane === "effort" ? (
          <>
            <Pressable onPress={() => setPane("telemetry")} accessibilityRole="button" accessibilityLabel="Back to session telemetry" style={styles.back}>
              <ArrowLeft size={16} strokeWidth={1.75} color={tokens.accent} />
              <Text style={[typeScale.sub, { color: tokens.accent }]}>Session telemetry</Text>
            </Pressable>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Reasoning effort</Text>
            <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>How intensely Forge reasons for this session.</Text>
            <View accessibilityRole="radiogroup" accessibilityLabel="Reasoning effort options">
              {[null, ...EFFORT_LEVELS].map((level, index) => {
                const selected = level === current;
                const label = level ?? "default";
                return (
                  <Pressable
                    key={label}
                    onPress={() => selectEffort(level)}
                    accessibilityRole="radio"
                    accessibilityState={{ selected }}
                    style={[styles.row, index > 0 ? { borderTopColor: tokens.hairline, borderTopWidth: StyleSheet.hairlineWidth } : null]}
                  >
                    <Text style={[styles.mono, { color: level === "whitehot" ? tokens.accent : selected ? tokens.ink : tokens.ink2 }]}>{label}</Text>
                    {selected ? <View style={[styles.dot, { backgroundColor: tokens.accent }]} /> : null}
                  </Pressable>
                );
              })}
            </View>
          </>
        ) : (
          <>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Session telemetry</Text>
            <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>Context, routing, and reasoning for this session.</Text>

            <SectionLabel>context</SectionLabel>
            {contextLimit != null ? (
              <View style={styles.block}>
                <View style={[styles.track, { backgroundColor: tokens.border }]}>
                  <View
                    style={[
                      styles.fill,
                      {
                        width: `${contextPercent ?? 0}%`,
                        backgroundColor: gaugeColor(contextPercent ?? 0, tokens),
                      },
                    ]}
                  />
                </View>
                <View style={styles.pair}>
                  <Text style={[styles.mono, tabularNums, { color: tokens.ink2 }]}>{formatTokenPair(contextTokens, contextLimit)}</Text>
                  <Text style={[styles.mono, tabularNums, { color: contextPercent != null && contextPercent >= 90 ? tokens.danger : tokens.ink3 }]}>
                    {`${contextPercent?.toFixed(0)}% used`}
                  </Text>
                </View>
              </View>
            ) : (
              <Text style={[typeScale.sub, styles.block, { color: tokens.ink3 }]}>Context capacity is unavailable for this model.</Text>
            )}
            <ActionRow label="Compact context" detail="summarize earlier conversation to free capacity" onPress={() => run("/compact")} />
            <ActionRow label="Restore compacted context" detail="bring back the previous full conversation" onPress={() => run("/uncompact")} quiet last />

            <SectionLabel>quota</SectionLabel>
            {weekly ? (
              <MetaRow label={`share of ${weekly.provider} weekly`} value={`+${weekly.deltaPct.toFixed(1)}%`} valueColor={tokens.success} last />
            ) : (
              <MetaRow label="metered API cost" value={formatCost(costUsd)} valueColor={tokens.success} last />
            )}

            <SectionLabel>model</SectionLabel>
            {tier ? <MetaRow label="tier" value={tier} /> : null}
            <NavRow label="Model" value={model} onPress={() => run("/model")} />
            <NavRow label="Operating mode" value={temper} onPress={() => run("/mode")} />
            <ActionRow label="Explain mesh routing" detail="see why Forge chose this model" onPress={() => run("/mesh")} last />

            <SectionLabel>reasoning effort</SectionLabel>
            <NavRow label="Effort" value={current ?? "default"} onPress={() => setPane("effort")} last />
          </>
        )}
      </View>
    </Sheet>
  );
}

function SectionLabel({ children }: { children: string }) {
  const tokens = useTokens();
  return <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>{children}</Text>;
}

function MetaRow({ label, value, valueColor, last = false }: { label: string; value: string; valueColor?: string; last?: boolean }) {
  const tokens = useTokens();
  return (
    <View style={[styles.row, !last ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
      <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{label}</Text>
      <Text style={[styles.mono, tabularNums, styles.rowValue, { color: valueColor ?? tokens.ink }]} numberOfLines={1}>
        {value}
      </Text>
    </View>
  );
}

function NavRow({ label, value, onPress, last = false }: { label: string; value: string; onPress: () => void; last?: boolean }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${label}: ${value}`}
      style={[styles.row, !last ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}
    >
      <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{label}</Text>
      <View style={styles.rowTrailing}>
        <Text style={[styles.mono, tabularNums, styles.rowValue, { color: tokens.ink3 }]} numberOfLines={1}>
          {value}
        </Text>
        <ChevronRight size={15} strokeWidth={1.75} color={tokens.ink4} />
      </View>
    </Pressable>
  );
}

function ActionRow({ label, detail, onPress, quiet = false, last = false }: { label: string; detail: string; onPress: () => void; quiet?: boolean; last?: boolean }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      style={[styles.actionRow, !last ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}
    >
      <Text style={[typeScale.bodyBold, { color: quiet ? tokens.ink2 : tokens.accent }]}>{label}</Text>
      <Text style={[typeScale.meta, { color: tokens.ink4 }]} numberOfLines={1}>
        {detail}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space32 },
  back: { alignSelf: "flex-start", minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space4 },
  subtitle: { marginTop: 2 },
  sectionLabel: { paddingTop: space.space20, paddingBottom: space.space4 },
  block: { paddingVertical: space.space8, gap: space.space8 },
  track: { height: 3, borderRadius: 2, overflow: "hidden" },
  fill: { height: "100%", borderRadius: 2 },
  pair: { flexDirection: "row", alignItems: "baseline", justifyContent: "space-between", gap: space.space8 },
  row: { minHeight: 48, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12 },
  rowTrailing: { flexDirection: "row", alignItems: "center", gap: space.space8, flexShrink: 1, minWidth: 0 },
  rowValue: { flexShrink: 1, textAlign: "right" },
  actionRow: { minHeight: 52, justifyContent: "center", gap: 2 },
  mono: { fontFamily: monoFamily.regular, fontSize: 12.5, lineHeight: 17 },
  dot: { width: 6, height: 6, borderRadius: 3 },
});
