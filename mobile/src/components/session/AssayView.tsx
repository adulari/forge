// Hearth Assay result language — presentational rendering of an assay report plus a parser
// that upgrades a recognizable assay report text block (the `◈ ASSAY REPORT …` plain form the
// headless runner prints, or the `## Forge Assay Report` markdown table) into the rich view.
//
// Honesty: assay results reach the mobile client ONLY as transcript/system-output text today.
// That text carries severity, confidence, category/lens, file:line, title, fix + effort, scope,
// run id, cost and the severity counts — it does NOT carry per-finding `verified`
// (CONFIRMED/refuted), voter counts, reviewer model ids, rationale, or an editorial verdict
// sentence. The typed props below model the full prototype language so a richer (live) source
// could drive them later, but `parseAssayReport` only populates what the text honestly carries
// and `AssayView` renders the rest conditionally (see the omissions note in the handoff report).
import { Check, ChevronDown, ChevronRight, ScanSearch, ShieldCheck, SkipForward } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { radii, space, type ColorTokens } from "../../theme/tokens";
import { formatCost, monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { StatusDot } from "../ds/StatusDot";
import { Button } from "../ds/Button";

// ---------------------------------------------------------------------------
// Typed props (no wire coupling)
// ---------------------------------------------------------------------------

export type AssaySeverity = "critical" | "high" | "medium" | "low";
export type AssayConfidence = "high" | "medium" | "low";
export type AssayVerdict = "confirmed" | "refuted";
export type ReviewerStatus = "running" | "done" | "skipped";

export interface AssayReviewer {
  lens: string;
  /** Model id (mono) — omitted when the source text doesn't carry it. */
  model?: string;
  status: ReviewerStatus;
  /** Reason a lens was skipped (graceful degradation). */
  skipReason?: string;
}

export interface AssayVoters {
  agree: number;
  total: number;
}

export interface AssayFinding {
  id: string;
  severity: AssaySeverity;
  title: string;
  file: string;
  line?: number;
  /** WHY it's a problem — shown inline when present (prototype HIGH row). */
  rationale?: string;
  /** Suggested fix + effort — revealed behind the row's expander. */
  fix?: string;
  effort?: string;
  category?: string;
  confidence?: AssayConfidence;
  /** Adversarial-pass outcome. Absent unless a richer source supplies it. */
  verdict?: AssayVerdict;
  /** Voter tally. Absent unless a richer source supplies it. */
  voters?: AssayVoters;
}

export interface AssayReportData {
  scopeLabel: string;
  runId?: string;
  costUsd?: number;
  /** Wall-clock label (e.g. "1m 44s") — client can't derive it from text; optional. */
  elapsedLabel?: string;
  running?: boolean;
  reviewers: AssayReviewer[];
  findings: AssayFinding[];
  findingCount: number;
  /** Editorial verdict sentence when a source supplies one; otherwise AssayView derives an
   * honest counts summary. */
  summary?: string;
  clean?: boolean;
}

export interface AssayAction {
  label: string;
  onPress: () => void;
  variant?: "primary" | "secondary";
}

// ---------------------------------------------------------------------------
// Small chips (handoff pattern 6 — severity / verdict / confidence)
// ---------------------------------------------------------------------------

function severityChipStyle(sev: AssaySeverity, tokens: ColorTokens): { bg: string; ink: string; label: string } {
  switch (sev) {
    case "critical":
      return { bg: tokens.dangerBg, ink: tokens.danger, label: "CRIT" };
    case "high":
      return { bg: tokens.dangerBg, ink: tokens.danger, label: "HIGH" };
    case "medium":
      return { bg: tokens.warnBg, ink: tokens.warnBgInk, label: "MED" };
    case "low":
    default:
      return { bg: tokens.bg3, ink: tokens.ink2, label: "LOW" };
  }
}

export function SeverityChip({ severity }: { severity: AssaySeverity }) {
  const tokens = useTokens();
  const { bg, ink, label } = severityChipStyle(severity, tokens);
  return (
    <View style={[styles.chip, { backgroundColor: bg }]}>
      <Text style={[styles.chipStrong, { color: ink }]}>{label}</Text>
    </View>
  );
}

export function VerdictChip({ verdict }: { verdict: AssayVerdict }) {
  const tokens = useTokens();
  if (verdict === "confirmed") {
    return (
      <View style={[styles.chip, { backgroundColor: tokens.successBg }]}>
        <Text style={[styles.chipMed, { color: tokens.success }]}>CONFIRMED</Text>
      </View>
    );
  }
  return (
    <View style={[styles.chip, styles.chipOutline, { borderColor: tokens.borderStrong }]}>
      <Text style={[styles.chipMed, { color: tokens.ink3 }]}>refuted</Text>
    </View>
  );
}

function confidenceChipStyle(conf: AssayConfidence, tokens: ColorTokens): { bg?: string; ink: string; outline?: boolean } {
  switch (conf) {
    case "high":
      return { bg: tokens.successBg, ink: tokens.success };
    case "medium":
      return { bg: tokens.warnBg, ink: tokens.warnBgInk };
    case "low":
    default:
      return { ink: tokens.ink3, outline: true };
  }
}

/** Right-aligned confidence pill — the honest stand-in for the prototype's verdict chip when
 * the source text carries post-verification confidence but not a confirmed/refuted verdict. */
export function ConfidenceChip({ confidence }: { confidence: AssayConfidence }) {
  const tokens = useTokens();
  const { bg, ink, outline } = confidenceChipStyle(confidence, tokens);
  return (
    <View style={[styles.chip, outline ? [styles.chipOutline, { borderColor: tokens.borderStrong }] : { backgroundColor: bg }]}>
      <Text style={[styles.chipMed, { color: ink }]}>{`${confidence} confidence`}</Text>
    </View>
  );
}

export function VoterDots({ voters }: { voters: AssayVoters }) {
  const tokens = useTokens();
  const dots = Array.from({ length: voters.total }, (_, i) => i < voters.agree);
  return (
    <View style={styles.voterRow}>
      <View style={styles.voterDots}>
        {dots.map((agree, i) => (
          <View key={i} style={[styles.voterDot, { backgroundColor: agree ? tokens.success : tokens.ink4 }]} />
        ))}
      </View>
      <Text style={[styles.monoTiny, tabularNums, { color: tokens.ink4 }]}>{`${voters.agree}/${voters.total} voters`}</Text>
    </View>
  );
}

// ---------------------------------------------------------------------------
// Reviewer fan-out chips
// ---------------------------------------------------------------------------

export function ReviewerChip({ reviewer }: { reviewer: AssayReviewer }) {
  const tokens = useTokens();
  const skipped = reviewer.status === "skipped";
  return (
    <View
      style={[styles.reviewerChip, { backgroundColor: tokens.bg3, opacity: skipped ? 0.7 : 1 }]}
      accessibilityRole="text"
      accessibilityLabel={
        skipped
          ? `${reviewer.lens} skipped${reviewer.skipReason ? `: ${reviewer.skipReason}` : ""}`
          : `${reviewer.lens} reviewer ${reviewer.status}`
      }
    >
      {reviewer.status === "done" ? <Check size={12} strokeWidth={2.5} color={tokens.success} /> : null}
      {reviewer.status === "running" ? <StatusDot state="busy" size={6} /> : null}
      {skipped ? <SkipForward size={12} strokeWidth={2} color={tokens.ink4} /> : null}
      <Text style={[typeScale.meta, { color: skipped ? tokens.ink3 : tokens.ink2 }]} numberOfLines={1}>
        {reviewer.model ? `${reviewer.lens} · ${reviewer.model}` : reviewer.lens}
      </Text>
    </View>
  );
}

function ReviewerFanout({ reviewers }: { reviewers: AssayReviewer[] }) {
  if (reviewers.length === 0) return null;
  return (
    <View style={styles.fanout}>
      {reviewers.map((r) => (
        <ReviewerChip key={`${r.lens}-${r.status}`} reviewer={r} />
      ))}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Finding row
// ---------------------------------------------------------------------------

export function FindingRow({ finding, compact }: { finding: AssayFinding; compact: boolean }) {
  const tokens = useTokens();
  const [expanded, setExpanded] = useState(false);
  const dim = finding.verdict === "refuted" || finding.confidence === "low";
  const detail = finding.fix || finding.category;
  const location = finding.line != null ? `${finding.file}:${finding.line}` : finding.file;

  // Right-aligned status chip: a real verdict if the source carries one, else the honest
  // confidence stand-in.
  const rightChip = finding.verdict ? (
    <VerdictChip verdict={finding.verdict} />
  ) : finding.confidence ? (
    <ConfidenceChip confidence={finding.confidence} />
  ) : null;

  return (
    <View style={[styles.finding, dim ? styles.dim : null]}>
      <Pressable
        onPress={detail ? () => setExpanded((v) => !v) : undefined}
        disabled={!detail}
        accessibilityRole={detail ? "button" : undefined}
        accessibilityState={detail ? { expanded } : undefined}
        accessibilityLabel={`${finding.severity} finding: ${finding.title}`}
        style={styles.findingHead}
        hitSlop={6}
      >
        <SeverityChip severity={finding.severity} />
        <Text style={[typeScale.bodyBold, styles.findingTitle, { color: dim ? tokens.ink2 : tokens.ink }]}>{finding.title}</Text>
        {/* Desktop keeps verdict + voters on the header line; compact drops voters to the evidence row. */}
        {rightChip}
        {!compact && finding.voters ? <VoterDots voters={finding.voters} /> : null}
        {detail ? (
          expanded ? (
            <ChevronDown size={15} strokeWidth={1.75} color={tokens.ink4} />
          ) : (
            <ChevronRight size={15} strokeWidth={1.75} color={tokens.ink4} />
          )
        ) : null}
      </Pressable>

      {finding.rationale ? (
        <Text style={[typeScale.sub, styles.rationale, { color: dim ? tokens.ink3 : tokens.ink2 }]}>{finding.rationale}</Text>
      ) : null}

      <View style={styles.evidenceRow}>
        <Text style={[styles.mono, { color: tokens.info }]} numberOfLines={1} selectable>
          {location}
        </Text>
        <View style={styles.grow} />
        {compact && finding.voters ? <VoterDots voters={finding.voters} /> : null}
      </View>

      {expanded && detail ? (
        <View style={styles.findingDetail}>
          {finding.fix ? (
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              <Text style={{ color: tokens.ink3 }}>fix · </Text>
              {finding.fix}
              {finding.effort ? <Text style={{ color: tokens.ink4 }}>{`  (${finding.effort})`}</Text> : null}
            </Text>
          ) : null}
          {finding.category ? (
            <Text style={[styles.monoTiny, { color: tokens.ink4 }]}>{finding.category}</Text>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

function FindingList({ findings, compact }: { findings: AssayFinding[]; compact: boolean }) {
  const tokens = useTokens();
  return (
    <View>
      {findings.map((f, i) => (
        <View key={f.id}>
          {i > 0 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
          <FindingRow finding={f} compact={compact} />
        </View>
      ))}
      <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} />
    </View>
  );
}

// ---------------------------------------------------------------------------
// Verdict summary card
// ---------------------------------------------------------------------------

export function VerdictCard({ report, actions }: { report: AssayReportData; actions?: AssayAction[] }) {
  const tokens = useTokens();
  const hasSevere = report.findings.some((f) => f.severity === "high" || f.severity === "critical");
  const edge = report.clean ? tokens.success : hasSevere ? tokens.danger : tokens.warn;
  const summary = report.summary ?? deriveSummary(report);
  return (
    <View style={[styles.verdictCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={[styles.verdictEdge, { backgroundColor: edge }]} />
      <View style={styles.verdictBody}>
        <Text style={[typeScale.section, { color: hasSevere && !report.clean ? tokens.danger : tokens.ink4 }]}>verdict</Text>
        <Text style={[typeScale.body, styles.verdictText, { color: tokens.ink }]}>{summary}</Text>
        {actions && actions.length > 0 ? (
          <View style={styles.actions}>
            {actions.map((a) => (
              <Button key={a.label} label={a.label} onPress={a.onPress} variant={a.variant ?? "primary"} />
            ))}
          </View>
        ) : null}
      </View>
    </View>
  );
}

function deriveSummary(report: AssayReportData): string {
  if (report.clean) return "No findings — clean, or the scope had no analyzable source.";
  const bySev = (s: AssaySeverity) => report.findings.filter((f) => f.severity === s).length;
  const crit = bySev("critical");
  const high = bySev("high");
  const med = bySev("medium");
  const low = bySev("low");
  const parts: string[] = [];
  if (crit) parts.push(`${crit} critical`);
  if (high) parts.push(`${high} high`);
  if (med) parts.push(`${med} medium`);
  if (low) parts.push(`${low} low`);
  const n = report.findingCount;
  const tail = parts.length > 0 ? ` · ${parts.join(" · ")}` : "";
  const lead = crit + high > 0 ? " — resolve the high-severity findings before merging." : ".";
  return `${n} finding${n === 1 ? "" : "s"}${tail}${lead}`;
}

// ---------------------------------------------------------------------------
// AssayView — full presentational renderer
// ---------------------------------------------------------------------------

export function AssayView({ report, actions }: { report: AssayReportData; actions?: AssayAction[] }) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const cost = report.costUsd != null ? formatCost(report.costUsd) : null;
  const meta = cost || report.elapsedLabel;

  const findingsSection = (
    <View>
      <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>{`findings · ${report.findingCount}`}</Text>
      {report.findings.length > 0 ? (
        <FindingList findings={report.findings} compact={isCompact} />
      ) : (
        <View style={styles.cleanRow}>
          <ScanSearch size={16} strokeWidth={1.75} color={tokens.success} />
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>No findings surfaced by the crew.</Text>
        </View>
      )}
    </View>
  );

  const verdictSection = (
    <View>
      {!isCompact ? (
        <>
          <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>verify · adversarial pass</Text>
          <Text style={[typeScale.sub, styles.verifyBlurb, { color: tokens.ink2 }]}>
            Verifier agents re-derive each finding from the diff alone; a finding survives only with a majority.
          </Text>
        </>
      ) : (
        <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>verdict</Text>
      )}
      <VerdictCard report={report} actions={actions} />
    </View>
  );

  return (
    <View style={styles.root} accessibilityLabel={`Assay report for ${report.scopeLabel}`}>
      <View style={styles.header}>
        {report.running ? <StatusDot state="busy" size={8} /> : <ShieldCheck size={18} strokeWidth={1.75} color={tokens.accent} />}
        <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          {`Assay · ${report.scopeLabel}`}
        </Text>
        {meta ? (
          <Text style={[styles.mono, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>
            {cost ? (
              <>
                <Text style={{ color: tokens.success }}>{cost}</Text>
                {report.elapsedLabel ? ` · ${report.elapsedLabel}` : ""}
              </>
            ) : (
              report.elapsedLabel
            )}
          </Text>
        ) : null}
      </View>

      <ReviewerFanout reviewers={report.reviewers} />

      {isCompact ? (
        <View style={styles.stack}>
          {findingsSection}
          {verdictSection}
        </View>
      ) : (
        <View style={styles.columns}>
          <View style={styles.colMain}>{findingsSection}</View>
          <View style={styles.colSide}>{verdictSection}</View>
        </View>
      )}
    </View>
  );
}

// ---------------------------------------------------------------------------
// Parser: recognizable assay report text → AssayReportData (or null → plain fallback)
// ---------------------------------------------------------------------------

const CATEGORY_LENS: Record<string, string> = {
  "dead-weight": "dead-weight",
  correctness: "correctness",
  unsafe: "safety",
  "test-coverage": "coverage",
  design: "design",
  architecture: "architecture",
  documentation: "docs",
  "over-engineering": "complexity",
};

function normalizeSeverity(raw: string): AssaySeverity | null {
  switch (raw.trim().toLowerCase()) {
    case "critical":
    case "crit":
      return "critical";
    case "high":
      return "high";
    case "medium":
    case "med":
      return "medium";
    case "low":
      return "low";
    default:
      return null;
  }
}

function normalizeConfidence(raw: string): AssayConfidence | undefined {
  switch (raw.trim().toLowerCase()) {
    case "high":
      return "high";
    case "medium":
    case "med":
      return "medium";
    case "low":
      return "low";
    default:
      return undefined;
  }
}

function reviewersFromFindings(
  findings: AssayFinding[],
  lensOf: (f: AssayFinding) => string | undefined,
  skipped: AssayReviewer[],
): AssayReviewer[] {
  const seen = new Set<string>();
  const done: AssayReviewer[] = [];
  for (const f of findings) {
    const lens = lensOf(f);
    if (!lens || seen.has(lens)) continue;
    seen.add(lens);
    done.push({ lens, status: "done" });
  }
  return [...done, ...skipped.filter((s) => !seen.has(s.lens))];
}

function parseSkipped(line: string): AssayReviewer[] {
  // "skipped: docs (rate limited), style (timeout)"
  const body = line.replace(/^skipped:\s*/i, "").trim();
  if (!body) return [];
  return body
    .split(/,\s*/)
    .map((chunk): AssayReviewer | null => {
      const trimmed = chunk.trim().replace(/;$/, "").trim();
      if (!trimmed) return null;
      const m = trimmed.match(/^(.+?)\s*\((.+)\)\s*$/);
      if (m) return { lens: m[1].trim(), status: "skipped", skipReason: m[2].trim() };
      return { lens: trimmed, status: "skipped" };
    })
    .filter((r): r is AssayReviewer => r !== null && r.lens.length > 0);
}

/** Parse the headless-runner plain form (`◈ ASSAY REPORT …`). */
function parsePlain(content: string): AssayReportData | null {
  const lines = content.split("\n");
  const headerIdx = lines.findIndex((l) => /◈\s*ASSAY REPORT/i.test(l));
  if (headerIdx === -1) return null;
  const header = lines[headerIdx].match(/◈\s*ASSAY REPORT\s+run\s+(\S+)\s+scope:\s*(.+?)\s*$/i);
  const runId = header?.[1];
  const scopeLabel = header?.[2]?.trim() || "repo";

  let costUsd: number | undefined;
  let findingCount = 0;
  let clean = false;
  let skipped: AssayReviewer[] = [];
  const findings: AssayFinding[] = [];

  const summaryRe = /^(\d+)\s+findings?\s+·\s+\d+\s+critical\s+·\s+\d+\s+high\s+·\s+\d+\s+medium\s+·\s+\d+\s+low\s+·\s+\$([0-9.]+)/i;
  const findingRe = /^\s*\d+\.\s*\[([^·\]]+)·([^\]]+)\]\s*(.+?)\s*—\s*(.+?)\s*$/;

  for (let i = headerIdx + 1; i < lines.length; i += 1) {
    const line = lines[i];
    const sm = line.match(summaryRe);
    if (sm) {
      findingCount = parseInt(sm[1], 10);
      costUsd = parseFloat(sm[2]);
      continue;
    }
    if (/^skipped:/i.test(line)) {
      skipped = parseSkipped(line);
      continue;
    }
    if (/no findings\s*—\s*clean/i.test(line)) {
      clean = true;
      continue;
    }
    const fm = line.match(findingRe);
    if (fm) {
      const severity = normalizeSeverity(fm[1]);
      if (!severity) continue;
      const confidence = normalizeConfidence(fm[2]);
      const category = fm[3].trim();
      const loc = fm[4].trim();
      const locM = loc.match(/^(.*):(\d+)$/);
      const file = locM ? locM[1] : loc;
      const lineNo = locM ? parseInt(locM[2], 10) : undefined;
      // The next indented line is the title; a following `fix:` line is the suggested fix.
      let title = category;
      let fix: string | undefined;
      let effort: string | undefined;
      for (let j = i + 1; j < lines.length && /^\s+\S/.test(lines[j]); j += 1) {
        const inner = lines[j];
        const fixM = inner.match(/^\s*fix:\s*(.+?)\s*\((\w+)\)\s*$/i);
        if (fixM) {
          fix = fixM[1].trim();
          effort = fixM[2].trim();
        } else if (title === category) {
          title = inner.trim();
        }
        i = j;
      }
      findings.push({
        id: `${file}:${lineNo ?? findings.length}`,
        severity,
        title,
        file,
        line: lineNo,
        fix,
        effort,
        category,
        confidence,
      });
    }
  }

  if (findings.length > 0) findingCount = findings.length;
  const reviewers = reviewersFromFindings(
    findings,
    (f) => (f.category ? CATEGORY_LENS[f.category] ?? f.category : undefined),
    skipped,
  );
  return { scopeLabel, runId, costUsd, running: false, reviewers, findings, findingCount, clean: clean && findings.length === 0 };
}

/** Parse the markdown table form (`## Forge Assay Report`). Carries lens but not confidence. */
function parseMarkdown(content: string): AssayReportData | null {
  if (!/##\s*Forge Assay Report/i.test(content)) return null;
  const lines = content.split("\n");
  let scopeLabel = "repo";
  let costUsd: number | undefined;
  let clean = false;
  let skipped: AssayReviewer[] = [];
  const findings: AssayFinding[] = [];

  for (const line of lines) {
    const scopeM = line.match(/\*\*Scope:\*\*\s*(.+?)\s{2,}\*\*Cost:\*\*\s*\$([0-9.]+)/i);
    if (scopeM) {
      scopeLabel = scopeM[1].trim();
      costUsd = parseFloat(scopeM[2]);
      continue;
    }
    if (/skipped lenses:/i.test(line)) {
      skipped = parseSkipped(line.replace(/^>?\s*\*\*Skipped lenses:\*\*/i, "skipped:").replace(/;/g, ","));
      continue;
    }
    if (/no findings\s*—\s*clean/i.test(line)) {
      clean = true;
      continue;
    }
    // | Severity | `location` | lens | title |
    const rowM = line.match(/^\|\s*(critical|high|medium|low)\s*\|\s*`([^`]+)`\s*\|\s*([^|]+?)\s*\|\s*(.+?)\s*\|\s*$/i);
    if (rowM) {
      const severity = normalizeSeverity(rowM[1]);
      if (!severity) continue;
      const loc = rowM[2].trim();
      const locM = loc.match(/^(.*):(\d+)$/);
      const lens = rowM[3].trim();
      findings.push({
        id: `${loc}-${findings.length}`,
        severity,
        title: rowM[4].replace(/\\\|/g, "|").trim(),
        file: locM ? locM[1] : loc,
        line: locM ? parseInt(locM[2], 10) : undefined,
        category: lens,
      });
    }
  }

  if (findings.length === 0 && !clean) return null;
  const reviewers = reviewersFromFindings(findings, (f) => f.category, skipped);
  return { scopeLabel, costUsd, running: false, reviewers, findings, findingCount: findings.length, clean: clean && findings.length === 0 };
}

/**
 * Parse a transcript/system-output block into a rich assay report, or return null when the text
 * isn't a recognizable assay report (the caller then falls back to plain-text rendering).
 */
export function parseAssayReport(content: string): AssayReportData | null {
  if (!content) return null;
  return parsePlain(content) ?? parseMarkdown(content);
}

/**
 * One-line hookup for the chat renderer: parse `content` and return a rich `<AssayView>` when it
 * is a recognizable assay report, otherwise null so the caller keeps its plain-text fallback.
 */
export function renderAssayReport(content: string): React.ReactElement | null {
  const report = parseAssayReport(content);
  return report ? <AssayView report={report} /> : null;
}

const styles = StyleSheet.create({
  root: { gap: space.space12, width: "100%" },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  fanout: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: space.space8 },
  stack: { gap: space.space16 },
  columns: { flexDirection: "row", flexWrap: "wrap", gap: space.space24 },
  colMain: { flexGrow: 1, flexBasis: 300, minWidth: 260 },
  colSide: { flexGrow: 1, flexBasis: 220, minWidth: 200 },
  label: { marginBottom: space.space4 },
  verifyBlurb: { marginBottom: space.space12 },

  // chips
  chip: { borderRadius: radii.radius4, paddingHorizontal: 7, paddingVertical: 2, alignSelf: "flex-start" },
  chipOutline: { borderWidth: StyleSheet.hairlineWidth, backgroundColor: "transparent" },
  chipStrong: { fontSize: 10, lineHeight: 14, fontWeight: "700", letterSpacing: 0.4 },
  chipMed: { fontSize: 10.5, lineHeight: 14, fontWeight: "600" },

  // reviewer chip
  reviewerChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    minHeight: 24,
    paddingHorizontal: space.space8,
    borderRadius: radii.radiusPill,
  },

  // finding
  finding: { paddingVertical: space.space12, gap: 5 },
  dim: { opacity: 0.75 },
  findingHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  findingTitle: { flex: 1 },
  rationale: {},
  evidenceRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  grow: { flex: 1 },
  findingDetail: { gap: space.space4, marginTop: 2 },
  hairline: { height: StyleSheet.hairlineWidth },

  // voters
  voterRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  voterDots: { flexDirection: "row", gap: 3 },
  voterDot: { width: 6, height: 6, borderRadius: 3 },

  // verdict card
  verdictCard: { position: "relative", borderRadius: radii.radius16, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  verdictEdge: { position: "absolute", left: 0, top: 0, bottom: 0, width: 2 },
  verdictBody: { padding: space.space16, gap: 6 },
  verdictText: { marginTop: 2 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, marginTop: space.space8 },

  cleanRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },

  mono: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 16 },
  monoTiny: { fontFamily: monoFamily.regular, fontSize: 10, lineHeight: 14 },
});
