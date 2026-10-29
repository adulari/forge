// Row-level rendering shared by every file card in the stacked diff: a single code line (with
// syntax colour and intraline change spans), a review-selectable line, a hunk header with its
// expand-context control, and the two-column split body. Changed lines are tinted with the
// design's exact alphas (add .09 success, del .08 danger) via `hexToRgba`, never a raw literal.
import { ChevronsUpDown, MessageSquare } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { type DiffCell, type SplitRow, type UnifiedRow } from "./diffModel";
import { styledSpans } from "./diffHighlight";
import { type TokenKind } from "../../lib/highlightTokens";
import { type ReviewCommentSide } from "../../lib/reviewComments";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { type ColorTokens, hexToRgba, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

export const LINE_HEIGHT = 19;
const LINE_NO_WIDTH = 30;

interface SplitBlock {
  key: string;
  header: string;
  pairs: Extract<SplitRow, { kind: "pair" }>[];
}

/** Hunk headers span both columns, so the flat row list is regrouped into per-hunk blocks —
 * each block renders one full-width header above its own two-column body. */
function toBlocks(rows: SplitRow[]): SplitBlock[] {
  const blocks: SplitBlock[] = [];
  for (const row of rows) {
    if (row.kind === "hunk") {
      blocks.push({ key: row.key, header: row.header, pairs: [] });
      continue;
    }
    blocks[blocks.length - 1]?.pairs.push(row);
  }
  return blocks;
}

function cellColors(kind: DiffCell["kind"], tokens: ColorTokens): { fill: string; ink: string; gutterInk: string } {
  if (kind === "add") {
    return { fill: hexToRgba(tokens.success, 0.09), ink: tokens.ink, gutterInk: tokens.success };
  }
  if (kind === "del") {
    return { fill: hexToRgba(tokens.danger, 0.08), ink: tokens.ink, gutterInk: tokens.danger };
  }
  return { fill: "transparent", ink: tokens.ink2, gutterInk: tokens.ink3 };
}

function tokenColor(kind: TokenKind, tokens: ColorTokens, keyword: string): { color: string; fontStyle?: "italic" } | undefined {
  switch (kind) {
    case "keyword":
      return { color: keyword };
    case "string":
      return { color: tokens.warn };
    case "number":
      return { color: tokens.info };
    case "comment":
      return { color: tokens.ink3, fontStyle: "italic" };
    default:
      return undefined;
  }
}

const DiffLine = React.memo(function DiffLine({
  cell,
  gutterChar,
  language,
}: {
  cell: DiffCell | null;
  gutterChar?: boolean;
  language: string | null;
}) {
  const { tokens, scheme } = useTheme();
  const keyword = scheme === "dark" ? tokens.ember.ember300 : tokens.ember.ember600;
  const spans = useMemo(
    () => (cell ? styledSpans(cell.text, language, cell.segments) : []),
    [cell, language],
  );
  if (!cell) {
    // The empty side of an unpaired change: a spacer, never a fabricated blank code line.
    return <View style={styles.line} />;
  }
  const { fill, ink, gutterInk } = cellColors(cell.kind, tokens);
  const prefix = cell.kind === "add" ? "+" : cell.kind === "del" ? "−" : " ";
  const changedFill = cell.kind === "add" ? hexToRgba(tokens.success, 0.24) : hexToRgba(tokens.danger, 0.22);
  return (
    <View style={[styles.line, { backgroundColor: fill }]}>
      <Text style={[styles.lineNo, tabularNums, { color: gutterInk }]}>{cell.lineNo}</Text>
      {gutterChar ? <Text style={[styles.gutterChar, { color: gutterInk }]}>{prefix}</Text> : null}
      <Text selectable style={[styles.lineText, { color: ink }]}>
        {cell.text.length === 0
          ? " "
          : spans.map((span, index) => (
              <Text
                key={`${index}:${span.kind}`}
                style={[tokenColor(span.kind, tokens, keyword), span.changed ? { backgroundColor: changedFill } : null]}
              >
                {span.text}
              </Text>
            ))}
      </Text>
    </View>
  );
});

export interface ReviewableLineProps {
  cell: DiffCell | null;
  gutterChar?: boolean;
  language: string | null;
  side: ReviewCommentSide;
  selected: boolean;
  commentCount: number;
  onPress: () => void;
}

export function ReviewableDiffLine({
  cell,
  gutterChar,
  language,
  side,
  selected,
  commentCount,
  onPress,
}: ReviewableLineProps) {
  const tokens = useTokens();
  if (!cell) return <DiffLine cell={null} gutterChar={gutterChar} language={language} />;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`Select ${side} line ${cell.lineNo} for review`}
      accessibilityState={{ selected }}
      style={[styles.reviewableLine, { borderLeftColor: selected ? tokens.accent : "transparent" }]}
    >
      <DiffLine cell={cell} gutterChar={gutterChar} language={language} />
      {selected ? (
        <View
          pointerEvents="none"
          style={[styles.selectionOverlay, { backgroundColor: hexToRgba(tokens.accent, 0.1) }]}
        />
      ) : null}
      {commentCount > 0 ? (
        <View pointerEvents="none" style={styles.commentMarker}>
          <MessageSquare size={11} strokeWidth={1.8} color={tokens.accent} />
          {commentCount > 1 ? (
            <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>{commentCount}</Text>
          ) : null}
        </View>
      ) : null}
    </Pressable>
  );
}

export function HunkHeader({
  header,
  onExpand,
}: {
  header: string;
  /** Shows the "more context" control; omitted when the file cannot grow any further. */
  onExpand?: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.hunkHeader, { borderTopColor: tokens.border }]}>
      <Text selectable style={[styles.lineText, styles.hunkText, { color: tokens.info }]} numberOfLines={1}>
        {header}
      </Text>
      {onExpand ? (
        <Pressable
          onPress={onExpand}
          accessibilityRole="button"
          accessibilityLabel="Show 20 more lines of context"
          hitSlop={6}
          style={styles.expand}
        >
          <ChevronsUpDown size={12} strokeWidth={1.75} color={tokens.ink3} />
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>20</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export interface DiffReviewCallbacks {
  isSelected: (side: ReviewCommentSide, lineNo: number) => boolean;
  commentCount: (side: ReviewCommentSide, lineNo: number) => number;
  onSelect: (side: ReviewCommentSide, cell: DiffCell) => void;
}

export function SplitBody({
  hunkRows,
  language,
  onExpand,
  isSelected,
  commentCount,
  onSelect,
}: { hunkRows: SplitRow[]; language: string | null; onExpand?: () => void } & DiffReviewCallbacks) {
  const tokens = useTokens();
  return (
    <>
      {toBlocks(hunkRows).map((block) => (
        <View key={block.key}>
          <HunkHeader header={block.header} onExpand={onExpand} />
          <View style={styles.splitRow}>
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              style={[styles.column, { borderRightColor: tokens.border }]}
              contentContainerStyle={styles.hscrollContent}
            >
              <View style={styles.columnBody}>
                {block.pairs.map((pair) =>
                  pair.left ? (
                    <ReviewableDiffLine
                      key={`l:${pair.key}`}
                      cell={pair.left}
                      language={language}
                      side="old"
                      selected={isSelected("old", pair.left.lineNo)}
                      commentCount={commentCount("old", pair.left.lineNo)}
                      onPress={() => onSelect("old", pair.left as DiffCell)}
                    />
                  ) : (
                    <DiffLine key={`l:${pair.key}`} cell={null} language={language} />
                  ),
                )}
              </View>
            </ScrollView>
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              style={styles.columnLast}
              contentContainerStyle={styles.hscrollContent}
            >
              <View style={styles.columnBody}>
                {block.pairs.map((pair) =>
                  pair.right ? (
                    <ReviewableDiffLine
                      key={`r:${pair.key}`}
                      cell={pair.right}
                      language={language}
                      side="new"
                      selected={isSelected("new", pair.right.lineNo)}
                      commentCount={commentCount("new", pair.right.lineNo)}
                      onPress={() => onSelect("new", pair.right as DiffCell)}
                    />
                  ) : (
                    <DiffLine key={`r:${pair.key}`} cell={null} language={language} />
                  ),
                )}
              </View>
            </ScrollView>
          </View>
        </View>
      ))}
    </>
  );
}

interface UnifiedBlock {
  key: string;
  header: string;
  lines: Extract<UnifiedRow, { kind: "line" }>[];
}

function toUnifiedBlocks(rows: UnifiedRow[]): UnifiedBlock[] {
  const blocks: UnifiedBlock[] = [];
  for (const row of rows) {
    if (row.kind === "hunk") {
      blocks.push({ key: row.key, header: row.header, lines: [] });
      continue;
    }
    blocks[blocks.length - 1]?.lines.push(row);
  }
  return blocks;
}

export function UnifiedBody({
  rows,
  language,
  onExpand,
  isSelected,
  commentCount,
  onSelect,
}: { rows: UnifiedRow[]; language: string | null; onExpand?: () => void } & DiffReviewCallbacks) {
  // One header and one horizontal scroller per hunk: a header inside the scroller would sit at
  // the far end of the longest line, with its expand control off-screen.
  return (
    <>
      {toUnifiedBlocks(rows).map((block) => (
        <View key={block.key}>
          <HunkHeader header={block.header} onExpand={onExpand} />
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.hscrollContent}>
            <View style={styles.columnBody}>
              {block.lines.map((row) => {
                const side: ReviewCommentSide = row.cell.kind === "del" ? "old" : "new";
                return (
                  <ReviewableDiffLine
                    key={row.key}
                    cell={row.cell}
                    gutterChar
                    language={language}
                    side={side}
                    selected={isSelected(side, row.cell.lineNo)}
                    commentCount={commentCount(side, row.cell.lineNo)}
                    onPress={() => onSelect(side, row.cell)}
                  />
                );
              })}
            </View>
          </ScrollView>
        </View>
      ))}
    </>
  );
}

const styles = StyleSheet.create({
  splitRow: { flexDirection: "row" },
  column: { flex: 1, borderRightWidth: StyleSheet.hairlineWidth },
  columnLast: { flex: 1 },
  // `minWidth: "100%"` on the body (and on each line) resolves against the horizontal scroll's
  // content container, which hugs the longest line unless it is told to fill the viewport: the
  // added/removed tint then stopped at the text instead of spanning the pane.
  hscrollContent: { minWidth: "100%" },
  columnBody: { minWidth: "100%", flexGrow: 1, paddingVertical: space.space4 },
  reviewableLine: { position: "relative", borderLeftWidth: 2 },
  selectionOverlay: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0 },
  commentMarker: {
    position: "absolute",
    right: space.space4,
    top: 3,
    minWidth: 16,
    height: 14,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 1,
  },
  line: {
    height: LINE_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 14,
    minWidth: "100%",
  },
  lineNo: { width: LINE_NO_WIDTH, fontSize: 10.5, lineHeight: 14, fontFamily: monoFamily.regular, textAlign: "right" },
  gutterChar: { fontSize: 11, lineHeight: 15, fontFamily: monoFamily.regular, width: 7 },
  lineText: { fontSize: 11, lineHeight: 15, fontFamily: monoFamily.regular },
  hunkHeader: {
    height: LINE_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space8,
    paddingHorizontal: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  hunkText: { flexShrink: 1 },
  expand: { flexDirection: "row", alignItems: "center", gap: 3 },
});
