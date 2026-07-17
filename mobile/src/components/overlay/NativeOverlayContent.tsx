// Hearth native overlay renderers. `overlay:mesh` becomes the "why this model" view
// (handoff pattern 5): the winner is a decision card with thin score bars, the rest are
// reject-reason rows (mono id + one-line reason, danger when benched), plus a fallback
// breadcrumb and budget meter parsed tolerantly from `overlay.body`. `overlay:workflow`
// rows speak the agent-row language (pattern 2): status medallion + running/waiting heat
// edge. `overlay:usage` is unchanged.
import { Check, Circle, Route, X } from "lucide-react-native";
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { Overlay, OverlayRow } from "../../lib/ws";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget, type ColorTokens } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Badge, type BadgeTone } from "../ds/Badge";
import { EmptyState } from "../ds/EmptyState";
import { HeatEdge } from "../ds/HeatEdge";
import { SectionHeader } from "../ds/SectionHeader";
import { CHECKPOINT_OVERLAY_KIND, CheckpointOverlayRows } from "../session/CheckpointSheet";
import { DuelOverlayRows, isDuelOverlayKind } from "../session/DuelView";

interface NativeOverlayRowsProps {
  overlay: Overlay;
  onSelect: (id: string) => void;
}

function selectedStyle(selected: boolean, selection: string) {
  return selected ? { backgroundColor: selection } : undefined;
}

// ---------------------------------------------------------------------------
// overlay:workflow — agent-row language (pattern 2)
// ---------------------------------------------------------------------------

function workflowState(label: string): "done" | "failed" | "running" | "pending" {
  const glyph = label.trimStart()[0];
  if (glyph === "✓") return "done";
  if (glyph === "✗") return "failed";
  if (glyph === "◐") return "running";
  return "pending";
}

function WorkflowMedallion({ state }: { state: ReturnType<typeof workflowState> }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot("busy");
  if (state === "done") return <Check size={16} strokeWidth={2} color={tokens.success} />;
  if (state === "failed") return <X size={16} strokeWidth={2} color={tokens.danger} />;
  if (state === "running") {
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
  if (overlay.rows.length === 0) return <EmptyState icon={Route} message="No workflow agents yet." />;
  let previousGroup: string | null = null;
  return (
    <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
      {overlay.rows.map((row) => {
        const showGroup = row.group != null && row.group !== previousGroup;
        previousGroup = row.group;
        const state = workflowState(row.label);
        const edge = state === "running" ? "busy" : state === "failed" ? "waiting" : false;
        return (
          <React.Fragment key={row.id}>
            {showGroup ? <SectionHeader>{row.group!}</SectionHeader> : null}
            <Pressable
              onPress={() => onSelect(row.id)}
              accessibilityRole="menuitem"
              accessibilityState={{ selected: row.selected }}
              accessibilityLabel={`${row.label} — ${row.detail}`}
              style={[styles.agentRow, selectedStyle(row.selected, tokens.selection)]}
            >
              {edge ? <HeatEdge state={edge} /> : null}
              <WorkflowMedallion state={state} />
              <View style={styles.rowCopy}>
                <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>{row.label.replace(/^[✓✗◐·]\s*/u, "")}</Text>
                {row.detail ? (
                  <Text style={[styles.agentTail, { color: state === "failed" ? tokens.danger : tokens.ink3 }]} numberOfLines={2}>{row.detail}</Text>
                ) : null}
              </View>
            </Pressable>
          </React.Fragment>
        );
      })}
    </ScrollView>
  );
}

// ---------------------------------------------------------------------------
// overlay:mesh — winner decision card + reject rows + budget + fallback chain
// ---------------------------------------------------------------------------

interface Score {
  label: string;
  value: number;
}

interface ParsedCandidate {
  id: string;
  scores: Score[];
  badges: string[];
  reason: string;
  benched: boolean;
}

const BADGE_KEYWORDS = new Set([
  "complex",
  "standard",
  "trivial",
  "subscription",
  "sub",
  "free",
  "api",
  "paid",
  "benched",
]);

function badgeTone(badge: string): BadgeTone {
  const b = badge.toLowerCase();
  if (b === "benched") return "danger";
  if (b === "complex" || b === "standard" || b === "trivial") return "warn";
  if (b === "subscription" || b === "sub") return "accent";
  if (b === "free") return "success";
  return "neutral";
}

function modelIdFrom(label: string): string {
  const rank = label.match(/^#\d+\s+(.+)$/);
  return (rank?.[1] ?? label).trim();
}

function parseCandidate(row: OverlayRow): ParsedCandidate {
  const id = modelIdFrom(row.label);
  const parts = row.detail.split(" · ").map((p) => p.trim()).filter(Boolean);
  const scores: Score[] = [];
  const badges: string[] = [];
  const reasonParts: string[] = [];
  for (const part of parts) {
    const score = part.match(/^([a-z][a-z ]{0,10}?)\s+(\d+(?:\.\d+)?)$/i);
    if (score) {
      scores.push({ label: `${score[1].trim()} ${score[2]}`, value: Number(score[2]) });
      continue;
    }
    if (/^[a-z0-9.:_+-]+$/i.test(part) && BADGE_KEYWORDS.has(part.toLowerCase())) {
      badges.push(part);
      continue;
    }
    reasonParts.push(part);
  }
  const reason = reasonParts.join(" · ");
  const benched = /bench|rate limit/i.test(`${reason} ${badges.join(" ")}`);
  return { id, scores, badges, reason, benched };
}

function ScoreBar({ score }: { score: Score }) {
  const tokens = useTokens();
  const width = Math.max(0, Math.min(100, score.value));
  return (
    <View style={styles.scoreRow}>
      <Text style={[styles.scoreLabel, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>{score.label}</Text>
      <View style={[styles.scoreTrack, { backgroundColor: tokens.border }]}>
        <View style={[styles.scoreFill, { width: `${width}%`, backgroundColor: tokens.accent }]} />
      </View>
    </View>
  );
}

function WinnerCard({ candidate, tokens }: { candidate: ParsedCandidate; tokens: ColorTokens }) {
  return (
    <View style={[styles.winnerCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <HeatEdge state="busy" />
      <View style={styles.winnerBody}>
        <View style={styles.winnerHead}>
          <Text style={[styles.winnerId, { color: tokens.ink }]} numberOfLines={1}>{candidate.id}</Text>
          {candidate.badges.map((badge) => (
            <Badge key={badge} label={badge} tone={badgeTone(badge)} shape="pill" />
          ))}
        </View>
        {candidate.reason ? <Text style={[typeScale.sub, styles.winnerReason, { color: tokens.ink2 }]}>{candidate.reason}</Text> : null}
        {candidate.scores.length > 0 ? <View style={styles.scores}>{candidate.scores.map((s) => <ScoreBar key={s.label} score={s} />)}</View> : null}
      </View>
    </View>
  );
}

function RejectRow({ candidate, onPress, selected }: { candidate: ParsedCandidate; onPress: () => void; selected: boolean }) {
  const tokens = useTokens();
  const reason = candidate.reason || candidate.badges.join(" · ");
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="menuitem"
      accessibilityState={{ selected }}
      accessibilityLabel={`${candidate.id} — ${reason}`}
      style={[styles.rejectRow, selectedStyle(selected, tokens.selection)]}
    >
      <View style={styles.rejectHead}>
        <Text style={[styles.rejectId, { color: tokens.ink2 }]} numberOfLines={1}>{candidate.id}</Text>
        {candidate.scores[0] ? <Text style={[styles.rejectScore, tabularNums, { color: tokens.ink3 }]}>{candidate.scores[0].label}</Text> : null}
        {candidate.benched ? <Text style={[typeScale.meta, { color: tokens.danger }]}>benched</Text> : null}
      </View>
      {reason ? <Text style={[typeScale.meta, styles.rejectReason, { color: candidate.benched ? tokens.danger : tokens.ink3 }]} numberOfLines={2}>{reason}</Text> : null}
    </Pressable>
  );
}

interface MeshExtras {
  budget: { used: number; total: number; raw: string } | null;
  chain: string[];
  leftover: string[];
}

function parseMeshBody(body: string | null): MeshExtras {
  const result: MeshExtras = { budget: null, chain: [], leftover: [] };
  if (body == null) return result;
  for (const raw of body.split("\n").map((l) => l.trim()).filter(Boolean)) {
    if (result.chain.length === 0 && /(→|->)/.test(raw)) {
      result.chain = raw
        .replace(/^[a-z ]*:/i, "")
        .split(/→|->/)
        .map((s) => s.trim())
        .filter(Boolean);
      continue;
    }
    const budget = raw.match(/\$\s*([\d.]+)\D+\$\s*([\d.]+)/);
    if (result.budget == null && budget) {
      result.budget = { used: Number(budget[1]), total: Number(budget[2]), raw };
      continue;
    }
    result.leftover.push(raw);
  }
  return result;
}

function BudgetMeter({ budget, tokens }: { budget: NonNullable<MeshExtras["budget"]>; tokens: ColorTokens }) {
  const pct = budget.total > 0 ? Math.max(0, Math.min(100, (budget.used / budget.total) * 100)) : 0;
  return (
    <>
      <SectionHeader>budget pressure</SectionHeader>
      <View style={styles.budgetRow}>
        <View style={[styles.scoreTrack, styles.budgetTrack, { backgroundColor: tokens.border }]}>
          <View style={[styles.scoreFill, { width: `${pct}%`, backgroundColor: pct >= 90 ? tokens.danger : tokens.accent }]} />
        </View>
        <Text style={[styles.budgetText, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>{budget.raw}</Text>
      </View>
    </>
  );
}

function FallbackChain({ chain, tokens }: { chain: string[]; tokens: ColorTokens }) {
  return (
    <>
      <SectionHeader>fallback chain</SectionHeader>
      <View style={styles.chainRow}>
        {chain.map((node, index) => (
          <React.Fragment key={`${node}-${index}`}>
            {index > 0 ? <Text style={[styles.chainArrow, { color: tokens.ink4 }]}>→</Text> : null}
            <Text style={[styles.chainNode, tabularNums, { color: index === 0 ? tokens.accent : tokens.ink2, fontWeight: index === 0 ? "700" : "400" }]}>{node}</Text>
          </React.Fragment>
        ))}
      </View>
    </>
  );
}

function MeshRows({ overlay, onSelect }: NativeOverlayRowsProps) {
  const tokens = useTokens();
  if (overlay.rows.length === 0) return <EmptyState icon={Route} message="No routing candidates to explain yet." />;

  const winnerIndex = Math.max(0, overlay.rows.findIndex((r) => r.selected));
  const winnerRow = overlay.rows[winnerIndex];
  const winner = parseCandidate(winnerRow);
  const rejects = overlay.rows.filter((_, i) => i !== winnerIndex);
  const extras = parseMeshBody(overlay.body);

  return (
    <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
      <WinnerCard candidate={winner} tokens={tokens} />

      <SectionHeader>candidates · ranked</SectionHeader>
      {rejects.map((row, index) => (
        <React.Fragment key={row.id}>
          {index > 0 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
          <RejectRow candidate={parseCandidate(row)} selected={row.selected} onPress={() => onSelect(row.id)} />
        </React.Fragment>
      ))}

      {extras.budget ? <BudgetMeter budget={extras.budget} tokens={tokens} /> : null}
      {extras.chain.length > 1 ? <FallbackChain chain={extras.chain} tokens={tokens} /> : null}
      {extras.leftover.length > 0 ? (
        <View style={styles.leftover}>
          {extras.leftover.map((line, index) => (
            <Text key={`${index}-${line}`} style={[typeScale.codeSmall, { color: tokens.ink3 }]}>{line}</Text>
          ))}
        </View>
      ) : null}
    </ScrollView>
  );
}

// ---------------------------------------------------------------------------
// overlay:usage — unchanged
// ---------------------------------------------------------------------------

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
  if (overlay.kind === CHECKPOINT_OVERLAY_KIND) return <CheckpointOverlayRows overlay={overlay} onSelect={onSelect} />;
  if (overlay.kind === "picker:duel") return <DuelOverlayRows overlay={overlay} onSelect={onSelect} />;
  return null;
}

export { NativeOverlayContent };

export function isNativeOverlayKind(kind: string): boolean {
  return kind === "overlay:workflow" || kind === "overlay:mesh" || kind === "overlay:usage" || kind === CHECKPOINT_OVERLAY_KIND || isDuelOverlayKind(kind);
}

const styles = StyleSheet.create({
  rows: { flex: 1 },
  agentRow: { minHeight: tapTarget, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  agentTail: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
  rowCopy: { flex: 1, gap: space.space2 },
  hairline: { height: StyleSheet.hairlineWidth },

  winnerCard: { position: "relative", marginTop: space.space8, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, overflow: "hidden" },
  winnerBody: { padding: space.space16, gap: space.space8 },
  winnerHead: { flexDirection: "row", alignItems: "center", gap: space.space8, flexWrap: "wrap" },
  winnerId: { flex: 1, minWidth: 120, fontFamily: monoFamily.bold, fontSize: 15, fontWeight: "700" },
  winnerReason: {},
  scores: { gap: space.space4, marginTop: space.space2 },
  scoreRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  scoreLabel: { fontFamily: monoFamily.regular, fontSize: 10.5, width: 76 },
  scoreTrack: { flex: 1, height: 3, borderRadius: 2, overflow: "hidden" },
  scoreFill: { height: "100%", borderRadius: 2 },

  rejectRow: { paddingVertical: space.space12, paddingHorizontal: space.space12, gap: space.space2 },
  rejectHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  rejectId: { flex: 1, fontFamily: monoFamily.bold, fontSize: 13, fontWeight: "700" },
  rejectScore: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  rejectReason: {},

  budgetRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingTop: space.space8 },
  budgetTrack: { flex: 1, height: 3 },
  budgetText: { fontFamily: monoFamily.regular, fontSize: 11, flexShrink: 1 },

  chainRow: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: space.space4, paddingHorizontal: space.space12, paddingTop: space.space8 },
  chainNode: { fontFamily: monoFamily.regular, fontSize: 11 },
  chainArrow: { fontFamily: monoFamily.regular, fontSize: 11 },

  leftover: { paddingHorizontal: space.space12, paddingTop: space.space12, gap: space.space2 },

  usageText: { paddingVertical: space.space2 },
  usageMeter: { gap: space.space4, paddingVertical: space.space8 },
  usageLabel: { flexDirection: "row", justifyContent: "space-between" },
  meterTrack: { height: 4, borderRadius: radii.radiusPill, overflow: "hidden" },
  meterFill: { height: "100%", borderRadius: radii.radiusPill },
});
