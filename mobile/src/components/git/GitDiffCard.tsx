// One file of the stacked diff: a collapsible card whose header carries the path, bucket and
// counts, and whose body is that file's diff (fetched only while the card is open). Split =
// old|new columns with per-side line numbers; unified = one column. A hunk header offers "20
// more lines of context", which refetches the file with a wider `context=`; the review-comment
// selection and sheet live per card because a selection belongs to one file.
//
// The body renders whatever `GET /api/git/diff` returned and nothing else: a binary file gets the
// daemon's one-line fact and no hunks, a rename shows both names, and a capped diff reports how
// many lines it left out instead of ending mid-file with no explanation.
import { ChevronDown, ChevronRight, MessageSquarePlus, X } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { type DiffBucket, type DiffEntry } from "./gitDiffEntries";
import { type DiffCell, middleTruncate, toSplitRows, toUnifiedRows } from "./diffModel";
import { languageForPath } from "./diffHighlight";
import { type DiffReviewCallbacks, SplitBody, UnifiedBody } from "./diffLines";
import { FileTypeIcon } from "./FileTypeIcon";
import { type GitDiffFile } from "../../lib/api";
import { useGitDiff } from "../../lib/queries";
import {
  buildReviewLineSelection,
  reviewDiffRevision,
  reviewRangeLabel,
  type ReviewCommentSide,
  type ReviewLineSelection,
  useReviewComments,
} from "../../lib/reviewComments";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { ReviewCommentSheet } from "../review/ReviewCommentSheet";

export type DiffViewMode = "split" | "unified";

const CONTEXT_STEP = 20;
const DEFAULT_CONTEXT = 3;
const MAX_CONTEXT = 2_000;
const MONO_ADVANCE = 0.6;
const HEADER_FONT_SIZE = 11.5;
/** Chevron + icon + bucket badge + counts + kind. */
const HEADER_CHROME_PX = 250;

const BUCKET_LABEL: Record<DiffBucket, string> = {
  staged: "STAGED",
  unstaged: "UNSTAGED",
  untracked: "NEW",
};

/** Comments attach to a file by a hash of its patch. Hashing only the changed lines (with their
 * line numbers) keeps a comment attached when the reviewer widens the context around it. */
function changeRevision(path: string, rows: ReturnType<typeof toUnifiedRows>): string {
  const lines: string[] = [];
  for (const row of rows) {
    if (row.kind === "line" && row.cell.kind !== "context") {
      lines.push(`${row.cell.kind === "add" ? "+" : "-"}${row.cell.lineNo}:${row.cell.text}`);
    }
  }
  return reviewDiffRevision(path, [{ header: "", lines }]);
}

export interface GitDiffCardProps {
  sessionId: string;
  entry: DiffEntry;
  expanded: boolean;
  /** The card the file list last pointed at. */
  focused: boolean;
  onToggle: (key: string) => void;
  mode: DiffViewMode;
  ignoreWhitespace: boolean;
  /** Width of the stack, for the path budget. */
  width: number;
  /** Reports the card's changed lines (and where each sits in the unified body) once loaded, for
   * the pane's Find. */
  onLines?: (key: string, lines: DiffFindLine[]) => void;
}

export interface DiffFindLine {
  lineNo: number;
  text: string;
  /** Position among the unified rows, hunk headers included. */
  row: number;
}

export const GitDiffCard = React.memo(function GitDiffCard({
  sessionId,
  entry,
  expanded,
  focused,
  onToggle,
  mode,
  ignoreWhitespace,
  width,
  onLines,
}: GitDiffCardProps) {
  const tokens = useTokens();
  const [context, setContext] = useState(0);
  const [exhausted, setExhausted] = useState(false);
  const [anchor, setAnchor] = useState<{ side: ReviewCommentSide; lineNo: number } | null>(null);
  const [lineSelection, setLineSelection] = useState<ReviewLineSelection | null>(null);
  const [commentVisible, setCommentVisible] = useState(false);
  const diff = useGitDiff(sessionId, entry.path, entry.staged, {
    context,
    ignoreWhitespace,
    enabled: expanded,
  });
  const file: GitDiffFile | null = diff.data?.files[0] ?? null;
  const reviewComments = useReviewComments(sessionId);
  const language = useMemo(() => languageForPath(entry.path), [entry.path]);
  const splitRows = useMemo(() => (file ? toSplitRows(file.hunks) : []), [file]);
  const unifiedRows = useMemo(() => (file ? toUnifiedRows(file.hunks) : []), [file]);
  const revision = useMemo(() => (file ? changeRevision(file.path, unifiedRows) : ""), [file, unifiedRows]);
  useEffect(() => {
    if (!onLines) return;
    const lines: DiffFindLine[] = [];
    unifiedRows.forEach((row, index) => {
      if (row.kind === "line") lines.push({ lineNo: row.cell.lineNo, text: row.cell.text, row: index });
    });
    onLines(entry.key, lines);
  }, [onLines, entry.key, unifiedRows]);
  const totalLines = file?.total_lines;
  const lastTotal = useRef<number | null>(null);

  // Widening the context only helps while it brings in more lines. Once a wider request returns
  // the same line count the file is fully shown, and the control goes away.
  useEffect(() => {
    if (totalLines == null || diff.isPlaceholderData) return;
    if (context > 0 && lastTotal.current != null && totalLines <= lastTotal.current) setExhausted(true);
    lastTotal.current = totalLines;
  }, [totalLines, context, diff.isPlaceholderData]);

  useEffect(() => {
    setContext(0);
    setExhausted(false);
    lastTotal.current = null;
  }, [entry.path, entry.staged, ignoreWhitespace]);

  useEffect(() => {
    setAnchor(null);
    setLineSelection(null);
    setCommentVisible(false);
  }, [entry.path, entry.staged]);

  const availableLines = useMemo(() => {
    const oldLines = new Map<number, DiffCell>();
    const newLines = new Map<number, DiffCell>();
    for (const row of splitRows) {
      if (row.kind !== "pair") continue;
      if (row.left) oldLines.set(row.left.lineNo, row.left);
      if (row.right) newLines.set(row.right.lineNo, row.right);
    }
    const plain = ({ lineNo, kind, text }: DiffCell) => ({ lineNo, kind, text });
    return { old: [...oldLines.values()].map(plain), new: [...newLines.values()].map(plain) };
  }, [splitRows]);
  const fileComments = useMemo(
    () =>
      reviewComments.filter(
        (comment) =>
          comment.path === file?.path && comment.revision === revision && comment.staged === entry.staged,
      ),
    [entry.staged, file?.path, reviewComments, revision],
  );

  const callbacks: DiffReviewCallbacks = {
    isSelected: (side, lineNo) =>
      lineSelection?.side === side && lineNo >= lineSelection.startLine && lineNo <= lineSelection.endLine,
    commentCount: (side, lineNo) =>
      fileComments.filter((comment) => comment.side === side && comment.endLine === lineNo).length,
    onSelect: (side, cell) => {
      const next = anchor?.side === side ? anchor : { side, lineNo: cell.lineNo };
      setAnchor(next);
      setLineSelection(buildReviewLineSelection(side, availableLines[side], next.lineNo, cell.lineNo));
    },
  };
  const clearSelection = () => {
    setAnchor(null);
    setLineSelection(null);
  };

  const canExpandContext =
    !exhausted &&
    (context || DEFAULT_CONTEXT) < MAX_CONTEXT &&
    file != null &&
    !file.binary &&
    file.kind !== "created" &&
    file.kind !== "deleted";
  const expandContext = canExpandContext
    ? () => setContext((current) => Math.min(MAX_CONTEXT, (current || DEFAULT_CONTEXT) + CONTEXT_STEP))
    : undefined;

  const adds = file?.adds ?? entry.row.adds;
  const dels = file?.dels ?? entry.row.dels;
  const binary = file?.binary ?? entry.row.binary;
  const pathBudget = Math.max(
    12,
    Math.floor((Math.max(width, 320) - HEADER_CHROME_PX) / (HEADER_FONT_SIZE * MONO_ADVANCE)),
  );
  const headerPath = entry.row.orig_path
    ? `${middleTruncate(entry.row.orig_path, Math.floor(pathBudget / 2))} → ${middleTruncate(entry.path, Math.floor(pathBudget / 2))}`
    : middleTruncate(entry.path, pathBudget);

  let body: React.ReactNode = null;
  if (expanded) {
    if (diff.isLoading && !file) {
      body = <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>Loading diff…</Text>;
    } else if (diff.error && !file) {
      body = <Text style={[typeScale.sub, styles.notice, { color: tokens.danger }]}>{diff.error.message}</Text>;
    } else if (!file) {
      body = (
        <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>
          {ignoreWhitespace && (entry.row.adds > 0 || entry.row.dels > 0)
            ? "Only whitespace changed in this file."
            : `No diff for this path ${entry.staged ? "in the index" : "in the working tree"}.`}
        </Text>
      );
    } else if (file.binary) {
      body = (
        <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>
          Binary file — no text diff. {file.kind}
          {file.orig_path ? `, renamed from ${file.orig_path}` : ""}.
        </Text>
      );
    } else if (file.hunks.length === 0) {
      body = (
        <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>
          {ignoreWhitespace && (entry.row.adds > 0 || entry.row.dels > 0)
            ? "Only whitespace changed in this file."
            : `${file.kind} with no content change (mode or metadata only).`}
        </Text>
      );
    } else {
      body = (
        <View>
          {lineSelection ? (
            <View style={[styles.selectionBar, { backgroundColor: tokens.bg2, borderBottomColor: tokens.border }]}>
              <MessageSquarePlus size={14} strokeWidth={1.8} color={tokens.accent} />
              <Text style={[typeScale.monoMeta, styles.selectionLabel, { color: tokens.ink2 }]}>
                {reviewRangeLabel(lineSelection.side, lineSelection.startLine, lineSelection.endLine)} ·{" "}
                {lineSelection.lines.length} line{lineSelection.lines.length === 1 ? "" : "s"}
              </Text>
              <Text style={[typeScale.monoMeta, styles.selectionHint, { color: tokens.ink4 }]}>
                tap another line to extend
              </Text>
              <Pressable
                onPress={() => setCommentVisible(true)}
                accessibilityRole="button"
                accessibilityLabel="Comment on selected lines"
                style={[styles.selectionAction, { borderColor: tokens.border, borderRadius: radii.radius4 }]}
              >
                <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>comment</Text>
              </Pressable>
              <Pressable
                onPress={clearSelection}
                accessibilityRole="button"
                accessibilityLabel="Clear selected lines"
                hitSlop={8}
                style={styles.selectionClose}
              >
                <X size={14} strokeWidth={1.8} color={tokens.ink3} />
              </Pressable>
            </View>
          ) : null}
          {mode === "split" ? (
            <SplitBody hunkRows={splitRows} language={language} onExpand={expandContext} {...callbacks} />
          ) : (
            <UnifiedBody rows={unifiedRows} language={language} onExpand={expandContext} {...callbacks} />
          )}
          {file.skipped_lines > 0 ? (
            <Text style={[typeScale.monoMeta, styles.notice, { color: tokens.ink4 }]}>
              Large diff — {file.skipped_lines.toLocaleString()}
              {totalLines ? ` of ${totalLines.toLocaleString()}` : ""} line{file.skipped_lines === 1 ? "" : "s"} not
              shown. The daemon caps one file&apos;s diff.
            </Text>
          ) : null}
        </View>
      );
    }
  }

  return (
    <View
      style={[
        styles.card,
        {
          borderColor: focused ? tokens.accent : tokens.border,
          backgroundColor: tokens.bg0,
          borderRadius: radii.radius8,
        },
      ]}
    >
      <Pressable
        onPress={() => onToggle(entry.key)}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        accessibilityLabel={`${expanded ? "Collapse" : "Expand"} ${entry.path}, ${BUCKET_LABEL[entry.bucket].toLowerCase()}`}
        style={[
          styles.header,
          { backgroundColor: tokens.bg1, borderBottomColor: tokens.border, borderBottomWidth: expanded ? StyleSheet.hairlineWidth : 0 },
        ]}
      >
        {expanded ? (
          <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
        ) : (
          <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
        )}
        <FileTypeIcon path={entry.path} />
        <Text style={[styles.headerPath, { color: tokens.ink }]} numberOfLines={1}>
          {headerPath}
        </Text>
        <View style={[styles.badge, { backgroundColor: hexToRgba(tokens.ink, 0.07) }]}>
          <Text style={[styles.badgeText, { color: tokens.ink3 }]}>{BUCKET_LABEL[entry.bucket]}</Text>
        </View>
        <View style={styles.spacer} />
        {binary ? (
          <Text style={[styles.headerMeta, { color: tokens.ink3 }]}>bin</Text>
        ) : (
          <>
            <Text style={[styles.headerMeta, tabularNums, { color: tokens.success }]}>+{adds}</Text>
            <Text style={[styles.headerMeta, tabularNums, { color: tokens.danger }]}>−{dels}</Text>
          </>
        )}
      </Pressable>
      {body}
      {expanded ? (
        <ReviewCommentSheet
          visible={commentVisible && file != null}
          sessionId={sessionId}
          path={file?.path ?? ""}
          revision={revision}
          staged={entry.staged}
          selection={lineSelection}
          onClose={() => setCommentVisible(false)}
          onAdded={clearSelection}
        />
      ) : null}
    </View>
  );
});

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  header: {
    minHeight: 32,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: space.space8,
  },
  headerPath: { fontSize: HEADER_FONT_SIZE, lineHeight: 16, fontFamily: monoFamily.regular, flexShrink: 1 },
  headerMeta: { fontSize: 9.5, lineHeight: 13, fontFamily: monoFamily.regular },
  spacer: { flex: 1 },
  badge: { paddingHorizontal: 5, paddingVertical: 1, borderRadius: 3 },
  badgeText: { fontSize: 8.5, lineHeight: 12, fontFamily: monoFamily.regular, letterSpacing: 0.4 },
  notice: { paddingHorizontal: space.space16, paddingVertical: space.space12 },
  selectionBar: {
    minHeight: 32,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  selectionLabel: { flexShrink: 0 },
  selectionHint: { flex: 1 },
  selectionAction: { borderWidth: StyleSheet.hairlineWidth, paddingHorizontal: space.space8, paddingVertical: 3 },
  selectionClose: { width: 24, height: 24, alignItems: "center", justifyContent: "center" },
});
