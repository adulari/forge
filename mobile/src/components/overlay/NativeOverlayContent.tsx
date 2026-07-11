import { Check, Circle, X } from "lucide-react-native";
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { Overlay } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { SectionHeader } from "../ds/SectionHeader";

interface NativeOverlayRowsProps {
  overlay: Overlay;
  onSelect: (id: string) => void;
}

function selectedStyle(selected: boolean, selection: string) {
  return selected ? { backgroundColor: selection } : undefined;
}

function WorkflowStatus({ label }: { label: string }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot("busy");
  const status = label.trimStart()[0];
  if (status === "✓") return <Check size={16} strokeWidth={2} color={tokens.success} />;
  if (status === "✗") return <X size={16} strokeWidth={2} color={tokens.danger} />;
  if (status === "◐") {
    return (
      <Animated.View style={dotStyle}>
        <Circle size={14} strokeWidth={2} color={tokens.accent} />
      </Animated.View>
    );
  }
  return <Circle size={14} strokeWidth={1.75} color={tokens.ink3} />;
}

function WorkflowRows({ overlay, onSelect }: NativeOverlayRowsProps) {
  const tokens = useTokens();
  let previousGroup: string | null = null;
  return (
    <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
      {overlay.rows.map((row) => {
        const showGroup = row.group != null && row.group !== previousGroup;
        previousGroup = row.group;
        return (
          <React.Fragment key={row.id}>
            {showGroup ? <SectionHeader>{row.group!}</SectionHeader> : null}
            <Pressable
              onPress={() => onSelect(row.id)}
              accessibilityRole="menuitem"
              accessibilityState={{ selected: row.selected }}
              accessibilityLabel={`${row.label} — ${row.detail}`}
              style={[styles.workflowRow, selectedStyle(row.selected, tokens.selection)]}
            >
              <WorkflowStatus label={row.label} />
              <View style={styles.rowCopy}>
                <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>{row.label.slice(2)}</Text>
                <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>{row.detail}</Text>
              </View>
            </Pressable>
          </React.Fragment>
        );
      })}
    </ScrollView>
  );
}

function MeshRows({ overlay, onSelect }: NativeOverlayRowsProps) {
  const tokens = useTokens();
  return (
    <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
      {overlay.rows.map((row) => {
        const rank = row.label.match(/^(#\d+)\s+(.+)$/);
        const [score, ...badges] = row.detail.split(" · ");
        return (
          <Pressable
            key={row.id}
            onPress={() => onSelect(row.id)}
            accessibilityRole="menuitem"
            accessibilityState={{ selected: row.selected }}
            accessibilityLabel={`${row.label} — ${row.detail}`}
            style={[styles.meshRow, selectedStyle(row.selected, tokens.selection)]}
          >
            <Text style={[typeScale.meta, styles.rank, { color: tokens.accent }]}>{rank?.[1] ?? "#"}</Text>
            <View style={styles.rowCopy}>
              <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>{rank?.[2] ?? row.label}</Text>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>{score}</Text>
              {badges.length > 0 ? <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>{badges.join(" · ")}</Text> : null}
            </View>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

function UsageBody({ body }: { body: string }) {
  const tokens = useTokens();
  return (
    <ScrollView style={styles.rows}>
      {body.split("\n").filter(Boolean).map((line, index) => {
        const percentage = line.match(/^(.*?):\s*(\d+(?:\.\d+)?)% used$/);
        if (!percentage) return <Text key={`${index}-${line}`} style={[typeScale.codeSmall, styles.usageText, { color: tokens.ink2 }]}>{line}</Text>;
        const pct = Math.max(0, Math.min(100, Number(percentage[2])));
        return (
          <View key={`${index}-${line}`} style={styles.usageMeter}>
            <View style={styles.usageLabel}><Text style={[typeScale.meta, { color: tokens.ink2 }]}>{percentage[1]}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{pct.toFixed(0)}%</Text></View>
            <View style={[styles.meterTrack, { backgroundColor: tokens.bg3 }]}><View style={[styles.meterFill, { width: `${pct}%`, backgroundColor: pct >= 90 ? tokens.danger : tokens.accent }]} /></View>
          </View>
        );
      })}
    </ScrollView>
  );
}

function NativeOverlayContent({ overlay, onSelect }: NativeOverlayRowsProps) {
  if (overlay.kind === "overlay:workflow") return <WorkflowRows overlay={overlay} onSelect={onSelect} />;
  if (overlay.kind === "overlay:mesh") return <MeshRows overlay={overlay} onSelect={onSelect} />;
  if (overlay.kind === "overlay:usage" && overlay.body != null) return <UsageBody body={overlay.body} />;
  return null;
}

export { NativeOverlayContent };

export function isNativeOverlayKind(kind: string): boolean {
  return kind === "overlay:workflow" || kind === "overlay:mesh" || kind === "overlay:usage";
}

const styles = StyleSheet.create({
  rows: { flex: 1 },
  workflowRow: { minHeight: tapTarget, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  meshRow: { minHeight: tapTarget, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  rank: { width: 28 },
  rowCopy: { flex: 1, gap: space.space2 },
  usageText: { paddingVertical: space.space2 },
  usageMeter: { gap: space.space4, paddingVertical: space.space8 },
  usageLabel: { flexDirection: "row", justifyContent: "space-between" },
  meterTrack: { height: 4, borderRadius: radii.radiusPill, overflow: "hidden" },
  meterFill: { height: "100%", borderRadius: radii.radiusPill },
});
