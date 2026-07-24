// Machined "Why this model" (Mobile/Desktop "Mesh Effort Duel"/"Mesh Explain" frames):
// picked-model card (name, tier/source pill badges, reasoning sentence, mono stat row) +
// a ranked candidate list with per-candidate reject reasons. Presentational — no wire
// coupling: the daemon already exposes this as an `overlay:mesh` Overlay, parsed by the
// shared pure helpers in `components/overlay/meshParse.ts`. `MeshExplainFromOverlay` bridges
// that live Overlay into this view's typed props and is mounted by
// `components/overlay/NativeOverlayContent.tsx`'s `MeshRows` as the overlay's body (which
// layers the budget meter + fallback chain parsed from `overlay.body` underneath it), the
// same pattern `DuelView.tsx` uses for `picker:duel` overlays.
import { Route as RouteIcon } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { Overlay } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { EmptyState } from "../ds/EmptyState";
import { badgeTone, parseCandidate, type ParsedCandidate } from "../overlay/meshParse";

export interface MeshExplainProps {
  /** The turn/breadcrumb caption (e.g. "/mesh · turn 14"). */
  turnLabel?: string;
  picked: ParsedCandidate | null;
  /** Ranked, excludes `picked`. Each carries the daemon overlay row id so taps can
   * round-trip as `overlay_select{id}` — the TUI moves its cursor/expansion to match. */
  candidates: (ParsedCandidate & { rowId?: string })[];
  /** Candidate-row tap → `overlay_select` sender. Omit for a read-only rendering. */
  onSelectRow?: (rowId: string) => void;
}

function StatRow({ scores }: { scores: ParsedCandidate["scores"] }) {
  const tokens = useTokens();
  if (scores.length === 0) return null;
  return (
    <View style={styles.stats}>
      {scores.map((s) => (
        <Text key={s.label} style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
          {s.label}
        </Text>
      ))}
    </View>
  );
}

function PickedCard({ candidate }: { candidate: ParsedCandidate }) {
  const tokens = useTokens();
  return (
    <View style={[styles.pickedCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={styles.pickedHead}>
        <Text style={[styles.modelId, { color: tokens.ink }]} numberOfLines={1}>
          {candidate.id}
        </Text>
        {candidate.badges.map((badge) => (
          <Badge key={badge} label={badge} tone={badgeTone(badge)} shape="pill" />
        ))}
      </View>
      {candidate.reason ? (
        <Text style={[typeScale.sub, styles.reason, { color: tokens.ink2 }]}>{candidate.reason}</Text>
      ) : null}
      <StatRow scores={candidate.scores} />
    </View>
  );
}

function CandidateRow({ candidate, onPress }: { candidate: ParsedCandidate; onPress?: () => void }) {
  const tokens = useTokens();
  const reason = candidate.reason || candidate.badges.join(" · ");
  const Row = onPress ? Pressable : View;
  return (
    <Row
      {...(onPress
        ? { onPress, accessibilityRole: "menuitem" as const, accessibilityLabel: `${candidate.id} — ${reason}` }
        : null)}
      style={styles.candidateRow}
    >
      <Text style={[styles.modelIdSmall, { color: tokens.ink }]} numberOfLines={1}>
        {candidate.id}
      </Text>
      {candidate.scores[0] ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          {candidate.scores[0].label}
        </Text>
      ) : null}
      <Text
        style={[typeScale.meta, styles.candidateReason, { color: candidate.benched ? tokens.danger : tokens.ink3 }]}
        numberOfLines={1}
      >
        {reason}
      </Text>
    </Row>
  );
}

export function MeshExplain({ turnLabel, picked, candidates, onSelectRow }: MeshExplainProps) {
  const tokens = useTokens();

  if (!picked && candidates.length === 0) {
    return <EmptyState icon={RouteIcon} message="No routing candidates to explain yet." />;
  }

  return (
    <View style={styles.root}>
      <View style={styles.header}>
        <Text style={[typeScale.title, styles.title, { color: tokens.ink }]}>Why this model</Text>
        {turnLabel ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            {turnLabel}
          </Text>
        ) : null}
      </View>

      {picked ? <PickedCard candidate={picked} /> : null}

      {candidates.length > 0 ? (
        <>
          <Text style={[typeScale.section, styles.candidatesLabel, { color: tokens.ink4 }]}>candidates · ranked</Text>
          <View>
            {candidates.map((c, index) => (
              <React.Fragment key={c.id}>
                {index > 0 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
                <CandidateRow
                  candidate={c}
                  onPress={onSelectRow && c.rowId != null ? () => onSelectRow(c.rowId as string) : undefined}
                />
              </React.Fragment>
            ))}
          </View>
        </>
      ) : null}
    </View>
  );
}

/** True when a live `Overlay.kind` is a mesh explanation this view can render. */
export function isMeshOverlayKind(kind: string): boolean {
  return kind === "overlay:mesh";
}

/** Bridges a live `overlay:mesh` Overlay into `MeshExplain`'s typed props — mirrors
 *  `NativeOverlayContent.tsx`'s `MeshRows` winner/reject split (the row the daemon marks
 *  `selected` is the picked model; the rest are ranked rejects). */
export function MeshExplainFromOverlay({
  overlay,
  turnLabel,
  onSelectRow,
}: {
  overlay: Overlay;
  turnLabel?: string;
  onSelectRow?: (rowId: string) => void;
}) {
  if (overlay.rows.length === 0) return <MeshExplain turnLabel={turnLabel} picked={null} candidates={[]} />;
  const pickedIndex = Math.max(0, overlay.rows.findIndex((r) => r.selected));
  const picked = parseCandidate(overlay.rows[pickedIndex]);
  const candidates = overlay.rows
    .filter((_, i) => i !== pickedIndex)
    .map((row) => ({ ...parseCandidate(row), rowId: row.id }));
  return <MeshExplain turnLabel={turnLabel} picked={picked} candidates={candidates} onSelectRow={onSelectRow} />;
}

const styles = StyleSheet.create({
  root: { gap: space.space8, width: "100%" },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  pickedCard: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    padding: space.space12,
    gap: space.space8,
  },
  pickedHead: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: space.space8 },
  modelId: { fontSize: 13, fontFamily: monoFamily.bold },
  modelIdSmall: { fontSize: 12, fontFamily: monoFamily.bold, flexShrink: 0 },
  reason: {},
  stats: { flexDirection: "row", gap: space.space12 },
  candidatesLabel: { marginTop: space.space8 },
  candidateRow: { flexDirection: "row", alignItems: "baseline", gap: space.space8, minHeight: 36, paddingVertical: space.space4 },
  candidateReason: { flex: 1, textAlign: "right" },
  hairline: { height: StyleSheet.hairlineWidth },
});
