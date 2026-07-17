// Hearth "Duel" comparison — dual-pane model arena (docs/features/duel.md). Presentational
// only: typed props, no wire coupling. Compact stacks the panes; medium+ shows them side by
// side (desktop prototype's 1fr 1fr grid). Each pane carries a mono model id, cost/latency,
// the response body (via the chat Markdown renderer) and a pick-winner action. Below the panes
// a scoreboard block draws thin win-rate bars (handoff pattern 10). The `DuelOverlayRows`
// export lets NativeOverlayContent render a live `picker:duel` overlay with this same view.
import { Swords } from "lucide-react-native";
import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { Overlay } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { EmptyState } from "../ds/EmptyState";
import { StatusDot } from "../ds/StatusDot";
import { Markdown } from "../chat/Markdown";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface DuelPane {
  /** Opaque id echoed back on pick (the candidate's worktree branch). */
  id: string;
  /** Model id, rendered mono (e.g. `claude-opus-4-8`, `codex-cli::gpt-5.5`). */
  model: string;
  costUsd: number;
  latencySec: number;
  /** Response body / summary — rendered through the chat Markdown component. */
  body: string;
  /** Header dot + pick-button emphasis. First pane is `accent`, the rest `info`. */
  tone?: "accent" | "info";
  diffstat?: { files: number; added: number; removed: number };
  /** `true` pass, `false` fail, `null`/undefined not run. */
  tests?: boolean | null;
}

export interface DuelScoreEntry {
  model: string;
  /** 0..1 — bar fill and the leading percentage. */
  winRate: number;
  wins: number;
  losses?: number;
}

export type DuelViewState = "ready" | "loading" | "error";

export interface DuelViewProps {
  /** The duelled task, shown mono beside the title. Empty hides the quote. */
  task?: string;
  panes: DuelPane[];
  onPick: (id: string) => void;
  scoreboard?: DuelScoreEntry[];
  /** Total duels behind the scoreboard (the `· 24 duels` count). */
  scoreboardTotal?: number;
  state?: DuelViewState;
  errorMessage?: string;
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

export function DuelView({
  task,
  panes,
  onPick,
  scoreboard,
  scoreboardTotal,
  state = "ready",
  errorMessage,
}: DuelViewProps) {
  const { isCompact } = useBreakpoint();

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={[styles.content, isCompact ? styles.contentCompact : styles.contentWide]}
      keyboardShouldPersistTaps="handled"
    >
      <View style={isCompact ? undefined : styles.wide}>
        <Header task={task} isCompact={isCompact} />

        {state === "error" ? (
          <View style={styles.stateBox}>
            <EmptyState icon={Swords} message={errorMessage ?? "The duel failed to produce a comparison."} />
          </View>
        ) : state === "loading" ? (
          <LoadingPanes isCompact={isCompact} />
        ) : panes.length === 0 ? (
          <View style={styles.stateBox}>
            <EmptyState icon={Swords} message="No usable candidates — the duel produced no answers to compare." />
          </View>
        ) : (
          <View style={[styles.panes, isCompact ? styles.panesStacked : styles.panesRow]}>
            {panes.map((pane, index) => (
              <PaneCard
                key={pane.id}
                pane={pane}
                index={index}
                isCompact={isCompact}
                onPick={() => onPick(pane.id)}
              />
            ))}
          </View>
        )}

        {scoreboard && scoreboard.length > 0 ? (
          <Scoreboard entries={scoreboard} total={scoreboardTotal} isCompact={isCompact} />
        ) : null}
      </View>
    </ScrollView>
  );
}

function Header({ task, isCompact }: { task?: string; isCompact: boolean }) {
  const tokens = useTokens();
  const trimmed = task?.trim();
  return (
    <View style={styles.header}>
      <Text style={[typeScale.headingBold, !isCompact && styles.titleWide, { color: tokens.ink }]}>Duel</Text>
      <Text style={[styles.mono, styles.taskQuote, { color: tokens.ink3 }]} numberOfLines={1}>
        {trimmed ? `"${trimmed}"${isCompact ? "" : " · pick the better answer"}` : "pick the better answer"}
      </Text>
    </View>
  );
}

function PaneCard({
  pane,
  index,
  isCompact,
  onPick,
}: {
  pane: DuelPane;
  index: number;
  isCompact: boolean;
  onPick: () => void;
}) {
  const tokens = useTokens();
  const accent = (pane.tone ?? (index === 0 ? "accent" : "info")) === "accent";
  const dotColor = accent ? tokens.accent : tokens.info;
  const meta = paneMeta(pane);
  return (
    <View style={[styles.pane, isCompact ? styles.paneStacked : styles.paneFlex, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={[styles.paneHead, { borderBottomColor: tokens.border }]}>
        <View style={[styles.paneDot, { backgroundColor: dotColor }]} />
        <Text style={[styles.monoBold, styles.paneModel, { color: tokens.ink }]} numberOfLines={1}>
          {pane.model}
        </Text>
        <Text style={[styles.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          <Text style={{ color: tokens.success }}>{formatCost(pane.costUsd)}</Text>
          {` · ${pane.latencySec.toFixed(1)}s`}
        </Text>
      </View>
      {meta ? (
        <Text style={[styles.monoMeta, tabularNums, styles.paneStat, { color: tokens.ink3 }]} numberOfLines={1}>
          {meta}
        </Text>
      ) : null}
      <View style={styles.paneBody}>
        <Markdown content={pane.body} style={styles.bodyText} />
      </View>
      <View style={styles.paneAction}>
        <PickButton accent={accent} label={isCompact ? "Pick this answer" : `Pick this answer · ${index + 1}`} onPress={onPick} />
      </View>
    </View>
  );
}

function PickButton({ accent, label, onPress }: { accent: boolean; label: string; onPress: () => void }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [
        styles.pick,
        accent
          ? { backgroundColor: tokens.accent }
          : { borderWidth: 1, borderColor: tokens.borderStrong },
        pressed && { opacity: 0.82 },
      ]}
    >
      <Text style={[typeScale.bodyBold, styles.pickLabel, { color: accent ? tokens.onAccent : tokens.ink2 }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

function LoadingPanes({ isCompact }: { isCompact: boolean }) {
  const tokens = useTokens();
  return (
    <View style={[styles.panes, isCompact ? styles.panesStacked : styles.panesRow]}>
      {[0, 1].map((i) => (
        <View
          key={i}
          style={[styles.pane, styles.paneLoading, isCompact ? styles.paneStacked : styles.paneFlex, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        >
          <StatusDot state="busy" />
          <Text style={[styles.mono, { color: tokens.ink3 }]}>racing model {i + 1}…</Text>
        </View>
      ))}
    </View>
  );
}

function Scoreboard({ entries, total, isCompact }: { entries: DuelScoreEntry[]; total?: number; isCompact: boolean }) {
  const tokens = useTokens();
  const heading = total != null ? `scoreboard · ${total} ${total === 1 ? "duel" : "duels"}` : "scoreboard";
  return (
    <View style={styles.scoreboard}>
      <Text style={[typeScale.section, { color: tokens.ink4 }]}>{heading}</Text>
      <View style={styles.scoreRows}>
        {entries.map((entry, index) => {
          const pct = Math.max(0, Math.min(1, entry.winRate));
          const record = entry.losses != null ? `${entry.wins}W ${entry.losses}L` : `${entry.wins}W`;
          return (
            <View key={entry.model} style={styles.scoreRow}>
              <Text style={[styles.mono, isCompact ? styles.scoreLabelCompact : styles.scoreLabelWide, { color: index === 0 ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>
                {entry.model}
              </Text>
              <View style={[styles.scoreTrack, { backgroundColor: tokens.border }]}>
                <View style={[styles.scoreFill, { width: `${pct * 100}%`, backgroundColor: tokens.accent }]} />
              </View>
              <Text style={[styles.monoMeta, tabularNums, styles.scoreValue, { color: tokens.ink3 }]} numberOfLines={1}>
                {`${Math.round(pct * 100)}% · ${record}`}
              </Text>
            </View>
          );
        })}
      </View>
      <Text style={[typeScale.meta, styles.scoreFoot, { color: tokens.ink4 }]}>Wins feed the mesh&apos;s per-task model rankings.</Text>
    </View>
  );
}

// ---------------------------------------------------------------------------
// Overlay bridge — render a live `picker:duel` overlay with the DuelView.
// The overlay rows carry only diffstat/tests/duration/cost/summary (see
// crates/forge-cli/.../pickers.rs::duel_picker_rows); the summary becomes the
// pane body and the scoreboard is omitted (the wire does not carry win history).
// ---------------------------------------------------------------------------

const DETAIL_DIFF_RE = /(\d+)\s+files?\s+\+(\d+)\s+-(\d+)/;

function panesFromOverlay(overlay: Overlay): DuelPane[] {
  return overlay.rows.map((row, index) => {
    const model = row.label.replace(/^[✓✗×]\s*/u, "").trim() || row.label;
    const parts = row.detail.split(" · ");
    const diffMatch = DETAIL_DIFF_RE.exec(parts[0] ?? "");
    const diffstat = diffMatch
      ? { files: Number(diffMatch[1]), added: Number(diffMatch[2]), removed: Number(diffMatch[3]) }
      : undefined;
    const tests = parts.some((p) => /tests\s*✓/.test(p))
      ? true
      : parts.some((p) => /tests\s*✗/.test(p))
        ? false
        : null;
    const latency = Number((parts.find((p) => /^\s*[\d.]+s$/.test(p)) ?? "0s").replace(/s\s*$/, "")) || 0;
    const cost = Number((parts.find((p) => /^\s*\$/.test(p)) ?? "$0").replace(/[^0-9.]/g, "")) || 0;
    const summary = parts.slice(4).join(" · ").trim();
    return {
      id: row.id,
      model,
      costUsd: cost,
      latencySec: latency,
      body: summary || "_(no summary provided)_",
      tone: index === 0 ? "accent" : "info",
      diffstat,
      tests,
    } satisfies DuelPane;
  });
}

/** True when `NativeOverlayContent` should hand a `picker:duel` overlay to `DuelOverlayRows`. */
export function isDuelOverlayKind(kind: string): boolean {
  return kind === "picker:duel";
}

/** Renders a live `/duel` winner-picker overlay as the dual-pane comparison view. */
export function DuelOverlayRows({ overlay, onSelect }: { overlay: Overlay; onSelect: (id: string) => void }) {
  const panes = React.useMemo(() => panesFromOverlay(overlay), [overlay]);
  return <DuelView panes={panes} onPick={onSelect} state="ready" />;
}

function paneMeta(pane: DuelPane): string | null {
  const bits: string[] = [];
  if (pane.diffstat) bits.push(`${pane.diffstat.files} files +${pane.diffstat.added} -${pane.diffstat.removed}`);
  if (pane.tests === true) bits.push("tests ✓");
  else if (pane.tests === false) bits.push("tests ✗");
  return bits.length ? bits.join(" · ") : null;
}

const styles = StyleSheet.create({
  scroll: { flex: 1 },
  content: { paddingVertical: space.space16 },
  contentCompact: { paddingHorizontal: space.space20 },
  contentWide: { paddingHorizontal: space.space32 },
  wide: { width: "100%", maxWidth: 1080, alignSelf: "center" },

  header: { flexDirection: "row", alignItems: "baseline", gap: space.space8, paddingVertical: space.space4 },
  titleWide: { fontSize: 19 },
  taskQuote: { flexShrink: 1 },

  panes: { marginTop: space.space12 },
  panesStacked: { gap: space.space12 },
  panesRow: { flexDirection: "row", gap: space.space20 },

  pane: { borderWidth: 1, borderRadius: radii.radius16, overflow: "hidden" },
  paneStacked: { width: "100%" },
  paneFlex: { flex: 1, minWidth: 0 },
  paneLoading: { alignItems: "center", justifyContent: "center", gap: space.space8, minHeight: 132 },

  paneHead: { flexDirection: "row", alignItems: "center", gap: space.space8, borderBottomWidth: 1, paddingHorizontal: space.space16, paddingVertical: space.space8 },
  paneDot: { width: 7, height: 7, borderRadius: radii.radiusPill },
  paneModel: { flex: 1 },
  paneStat: { paddingHorizontal: space.space16, paddingTop: space.space8 },
  paneBody: { paddingHorizontal: space.space16, paddingTop: space.space4 },
  bodyText: { fontSize: 13.5, lineHeight: 20 },
  paneAction: { paddingHorizontal: space.space16, paddingTop: space.space8, paddingBottom: space.space12 },

  pick: { height: 38, borderRadius: radii.radiusSegmentOuter, alignItems: "center", justifyContent: "center", paddingHorizontal: space.space12 },
  pickLabel: { fontSize: 13 },

  stateBox: { marginTop: space.space12, minHeight: 180 },

  scoreboard: { marginTop: space.space24 },
  scoreRows: { marginTop: space.space8 },
  scoreRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: 40 },
  scoreLabelCompact: { width: 118 },
  scoreLabelWide: { width: 150 },
  scoreTrack: { flex: 1, height: 4, borderRadius: radii.radiusPill, overflow: "hidden" },
  scoreFill: { height: "100%", borderRadius: radii.radiusPill },
  scoreValue: { minWidth: 78, textAlign: "right" },
  scoreFoot: { marginTop: space.space8 },

  mono: { fontFamily: monoFamily.regular, fontSize: 12, lineHeight: 17 },
  monoBold: { fontFamily: monoFamily.bold, fontSize: 12.5, lineHeight: 17 },
  monoMeta: { fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: 15 },
});
