// DESIGN_SYSTEM.md §6 DiffCard: per Snapshot.diff — `pending` variant gets a warn
// banner "proposed change — review before allowing"; collapsible file sections
// (chevron), header path (mono head-ellipsis), kind badge, `+a -d` tabular
// (success/danger); hunk header info-color mono; lines mono `codeSmall` with
// successBg/dangerBg full-width fills; "+N more lines/files" ink3 footers.
//
// Used both standalone in the Review segment (any diff, pending or landed) and
// embedded inside PermissionCard when `diff.pending` (FEATURES.md §1.2).
import * as Clipboard from "expo-clipboard";
import { Check, ChevronDown, ChevronRight, Copy, MessageSquarePlus, X } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { DiffLines } from "./DiffLines";
import { ReviewCommentSheet } from "./ReviewCommentSheet";
import { type DiffCell, toUnifiedRows } from "../git/diffModel";
import { Badge, type BadgeTone } from "../ds/Badge";
import { Banner } from "../ds/Banner";
import { IconButton } from "../ds/IconButton";
import { type Diff, type DiffFile } from "../../lib/ws";
import {
  buildReviewLineSelection,
  reviewDiffRevision,
  reviewRangeLabel,
  type ReviewCommentSource,
  type ReviewCommentSide,
  type ReviewLineSelection,
  useReviewComments,
} from "../../lib/reviewComments";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export interface DiffCardProps {
  diff: Diff;
  /** Enables line annotations for this session. Omitted in context-free embedded permission cards. */
  sessionId?: string;
  reviewSource?: Extract<ReviewCommentSource, "turn" | "fork">;
  /** Caps the card at this height with its own internal ScrollView, so whatever sits below it
   * (PermissionCard's Allow/Deny bar) never gets pushed off-screen by a large diff. Omitted
   * (full height, no internal scroll) on the standalone Review screen, which is already
   * scrollable end-to-end — this only matters where DiffCard is embedded in a non-scrolling
   * slot (FEATURES.md §1.2). */
  maxHeight?: number;
}

const HEAD_ELLIPSIS_MAX = 42;
const COPY_RESET_MS = 1200;

/** Mono "head-ellipsis": keeps the tail of a long path, prefixed with an ellipsis. */
function headEllipsis(path: string, max: number = HEAD_ELLIPSIS_MAX): string {
  if (path.length <= max) return path;
  return `…${path.slice(-(max - 1))}`;
}

function kindTone(kind: DiffFile["kind"]): BadgeTone {
  switch (kind) {
    case "created":
      return "success";
    case "deleted":
      return "danger";
    case "modified":
    default:
      return "neutral";
  }
}

export function DiffCard({ diff, sessionId, reviewSource = "turn", maxHeight }: DiffCardProps) {
  const tokens = useTokens();

  const body = (
    <>
      {diff.pending ? (
        <Banner tone="warn" message="proposed change — review before allowing" />
      ) : null}

      {diff.files.map((file, idx) => (
        <DiffFileSection
          key={`${file.path}-${idx}`}
          file={file}
          sessionId={sessionId}
          reviewSource={reviewSource}
          isLast={idx === diff.files.length - 1}
        />
      ))}

      {diff.skipped_files > 0 ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>
          +{diff.skipped_files} more file{diff.skipped_files === 1 ? "" : "s"}
        </Text>
      ) : null}
    </>
  );

  return (
    <View style={[styles.container, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      {maxHeight != null ? (
        <ScrollView style={{ maxHeight }} nestedScrollEnabled showsVerticalScrollIndicator>
          {body}
        </ScrollView>
      ) : (
        body
      )}
    </View>
  );
}

function DiffFileSection({
  file,
  sessionId,
  reviewSource,
  isLast,
}: {
  file: DiffFile;
  sessionId?: string;
  reviewSource: Extract<ReviewCommentSource, "turn" | "fork">;
  isLast: boolean;
}) {
  const tokens = useTokens();
  const [expanded, setExpanded] = useState(true);
  const [copied, setCopied] = useState(false);
  const [selectionAnchor, setSelectionAnchor] = useState<{
    side: ReviewCommentSide;
    lineNo: number;
  } | null>(null);
  const [lineSelection, setLineSelection] = useState<ReviewLineSelection | null>(null);
  const [commentVisible, setCommentVisible] = useState(false);
  const reviewComments = useReviewComments(sessionId ?? "");
  const fileRevision = useMemo(
    () => reviewDiffRevision(file.path, file.hunks),
    [file.hunks, file.path],
  );
  const unifiedRows = useMemo(() => toUnifiedRows(file.hunks), [file.hunks]);
  const availableLines = useMemo(() => {
    const oldLines = new Map<number, DiffCell>();
    const newLines = new Map<number, DiffCell>();
    unifiedRows.forEach((row) => {
      if (row.kind !== "line") return;
      const target = row.cell.kind === "del" ? oldLines : newLines;
      target.set(row.cell.lineNo, row.cell);
    });
    return {
      old: [...oldLines.values()].map(({ lineNo, kind, text }) => ({ lineNo, kind, text })),
      new: [...newLines.values()].map(({ lineNo, kind, text }) => ({ lineNo, kind, text })),
    };
  }, [unifiedRows]);
  const fileComments = useMemo(
    () =>
      reviewComments.filter(
        (comment) =>
          comment.source === reviewSource &&
          comment.path === file.path &&
          comment.revision === fileRevision,
      ),
    [file.path, fileRevision, reviewComments, reviewSource],
  );
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => {
    if (resetTimer.current) clearTimeout(resetTimer.current);
  }, []);
  useEffect(() => {
    setSelectionAnchor(null);
    setLineSelection(null);
    setCommentVisible(false);
  }, [file.path]);

  const selectLine = (side: ReviewCommentSide, cell: DiffCell) => {
    if (!sessionId) return;
    const anchor =
      selectionAnchor?.side === side ? selectionAnchor : { side, lineNo: cell.lineNo };
    setSelectionAnchor(anchor);
    setLineSelection(
      buildReviewLineSelection(side, availableLines[side], anchor.lineNo, cell.lineNo),
    );
  };
  const isSelected = (side: ReviewCommentSide, lineNo: number) =>
    lineSelection?.side === side &&
    lineNo >= lineSelection.startLine &&
    lineNo <= lineSelection.endLine;
  const commentCount = (side: ReviewCommentSide, lineNo: number) =>
    fileComments.filter((comment) => comment.side === side && comment.endLine === lineNo).length;
  const clearSelection = () => {
    setSelectionAnchor(null);
    setLineSelection(null);
  };

  const onCopy = async () => {
    const patch = file.hunks.map((h) => [h.header, ...h.lines].join("\n")).join("\n");
    await Clipboard.setStringAsync(patch);
    setCopied(true);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopied(false), COPY_RESET_MS);
  };

  return (
    <View style={[!isLast && styles.fileDivider, { borderBottomColor: tokens.border }]}>
      <View style={styles.fileHeaderRow}>
        <Pressable
          onPress={() => setExpanded((v) => !v)}
          accessibilityRole="button"
          accessibilityLabel={`${expanded ? "collapse" : "expand"} ${file.path}`}
          accessibilityState={{ expanded }}
          style={styles.fileHeader}
          hitSlop={8}
        >
          {expanded ? (
            <ChevronDown size={16} strokeWidth={1.75} color={tokens.ink3} />
          ) : (
            <ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />
          )}
          <Text
            selectable
            style={[typeScale.bodyBold, { color: tokens.ink, fontFamily: monoFamily.regular }, styles.filePath]}
            numberOfLines={1}
          >
            {headEllipsis(file.path)}
          </Text>
          <Badge label={file.kind} tone={kindTone(file.kind)} />
          {!file.binary ? (
            <Text style={[typeScale.meta, styles.counts]}>
              <Text style={{ color: tokens.success, fontFamily: monoFamily.regular }}>+{file.adds}</Text>
              {" "}
              <Text style={{ color: tokens.danger, fontFamily: monoFamily.regular }}>-{file.dels}</Text>
            </Text>
          ) : null}
        </Pressable>
        {!file.binary ? (
          <IconButton
            accessibilityLabel={copied ? "copied" : "copy patch"}
            onPress={onCopy}
            icon={
              copied ? (
                <Check size={20} color={tokens.success} strokeWidth={1.75} />
              ) : (
                <Copy size={20} color={tokens.ink3} strokeWidth={1.75} />
              )
            }
          />
        ) : null}
      </View>

      {lineSelection ? (
        <View
          style={[
            styles.selectionBar,
            { backgroundColor: tokens.bg2, borderColor: tokens.border },
          ]}
        >
          <MessageSquarePlus size={14} strokeWidth={1.8} color={tokens.accent} />
          <Text style={[typeScale.monoMeta, styles.selectionLabel, { color: tokens.ink2 }]}>
            {reviewRangeLabel(
              lineSelection.side,
              lineSelection.startLine,
              lineSelection.endLine,
            )}{" "}
            · {lineSelection.lines.length} line{lineSelection.lines.length === 1 ? "" : "s"}
          </Text>
          <Text style={[typeScale.monoMeta, styles.selectionHint, { color: tokens.ink4 }]}>
            tap another line to extend
          </Text>
          <Pressable
            onPress={() => setCommentVisible(true)}
            accessibilityRole="button"
            accessibilityLabel="Comment on selected turn diff lines"
            style={[
              styles.selectionAction,
              { borderColor: tokens.border, borderRadius: radii.radius4 },
            ]}
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

      {expanded && !file.binary ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={[styles.hunkScroll, styles.horizontalScroll]}>
          <View>
            {file.hunks.map((hunk, hIdx) => (
              <View key={hIdx} style={styles.hunk}>
                <Text selectable style={[typeScale.codeSmall, { color: tokens.info }, styles.hunkHeader]}>{hunk.header}</Text>
                <DiffLines
                  lines={hunk.lines}
                  header={hunk.header}
                  isSelected={sessionId ? isSelected : undefined}
                  commentCount={sessionId ? commentCount : undefined}
                  onSelect={sessionId ? selectLine : undefined}
                />
              </View>
            ))}
            {file.skipped_lines > 0 ? (
              <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>
                +{file.skipped_lines} more line{file.skipped_lines === 1 ? "" : "s"}
              </Text>
            ) : null}
          </View>
        </ScrollView>
      ) : expanded && file.binary ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.footer]}>binary file</Text>
      ) : null}
      <ReviewCommentSheet
        visible={commentVisible && sessionId != null}
        sessionId={sessionId ?? ""}
        path={file.path}
        revision={fileRevision}
        source={reviewSource}
        staged={false}
        selection={lineSelection}
        onClose={() => setCommentVisible(false)}
        onAdded={clearSelection}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, overflow: "hidden" },
  fileDivider: { borderBottomWidth: StyleSheet.hairlineWidth },
  fileHeaderRow: { flexDirection: "row", alignItems: "center" },
  fileHeader: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    minHeight: 44,
  },
  filePath: { flex: 1 },
  counts: { flexShrink: 0 },
  selectionBar: {
    minHeight: 34,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: space.space12,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  selectionLabel: { flexShrink: 0 },
  selectionHint: { flex: 1 },
  selectionAction: {
    borderWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: space.space8,
    paddingVertical: 3,
  },
  selectionClose: { width: 24, height: 24, alignItems: "center", justifyContent: "center" },
  hunkScroll: { marginBottom: space.space4 },
  horizontalScroll: { flexGrow: 0, flexShrink: 0 },
  hunk: { paddingBottom: space.space8 },
  hunkHeader: { paddingHorizontal: space.space12, paddingVertical: space.space4 },
  footer: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
});
