// One file's diff, virtualized. Every row is a fixed height so a 5,000-line diff costs a list of
// numbers, and only the rows near the viewport mount. Built on diffModel/diffHighlight (the same
// pairing and syntax rules the Review dock uses).
//
// PR C hook points: `renderHunkActions` draws accept / revert / comment on each hunk header, and
// `onSelectLine` receives a click on a line for inline comments. Both are optional and unused here.
import { FileDiff } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef } from "react";
import { type FlatList, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import type { GitDiffFile } from "../../lib/api";
import { useGitDiff, useGitStatus, useSessionDiff } from "../../lib/queries";
import { type TokenKind } from "../../lib/highlightTokens";
import { useTheme } from "../../theme/ThemeProvider";
import { type ColorTokens, hexToRgba, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor, tabularNums } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import type { DiffCell } from "../git/diffModel";
import { languageForPath, styledSpans } from "../git/diffHighlight";
import { registerDiffController } from "./diffController";
import { DIFF_LINE_HEIGHT, HUNK_HEIGHT, buildDiffRows, hunkTarget, type DiffMode, type DiffRow } from "./diffRows";

const SPLIT_BREAKPOINT = 900;

export interface DiffViewProps {
  sessionId: string;
  path: string;
  /** The session owns a worktree: its diff is a whole-branch diff rather than a git index diff. */
  worktree: boolean;
  mode?: DiffMode;
  renderHunkActions?: (hunk: number, header: string) => React.ReactNode;
  onSelectLine?: (side: "old" | "new", cell: DiffCell) => void;
  onLoaded?: (file: GitDiffFile | null) => void;
}

export function useFileDiff(sessionId: string, path: string, worktree: boolean) {
  const diff = useSessionDiff(sessionId, worktree);
  const status = useGitStatus(worktree ? null : sessionId);
  // A path can sit in both buckets; the working-tree side is what an agent just wrote.
  const staged = !!status.data && !status.data.unstaged.some((row) => row.path === path) && !status.data.untracked.some((row) => row.path === path) && status.data.staged.some((row) => row.path === path);
  const gitDiff = useGitDiff(sessionId, path, staged, { context: 3, enabled: !worktree && status.isSuccess });
  if (worktree) {
    return {
      file: diff.data?.files.find((file) => file.path === path) ?? null,
      loading: diff.isPending,
      error: diff.error,
    };
  }
  return {
    file: gitDiff.data?.files.find((file) => file.path === path) ?? gitDiff.data?.files[0] ?? null,
    loading: status.isPending || gitDiff.isPending,
    error: status.error ?? gitDiff.error,
  };
}

export function DiffView({ sessionId, path, worktree, mode: forcedMode, renderHunkActions, onSelectLine, onLoaded }: DiffViewProps) {
  const { file, loading, error } = useFileDiff(sessionId, path, worktree);
  const { width } = useWindowDimensions();
  const mode: DiffMode = forcedMode ?? (width >= SPLIT_BREAKPOINT + 560 ? "split" : "unified");
  const language = useMemo(() => languageForPath(path), [path]);
  const built = useMemo(() => buildDiffRows(file?.hunks ?? [], mode), [file, mode]);
  const listRef = useRef<FlatList<DiffRow>>(null);
  const firstRow = useRef(0);

  useEffect(() => {
    onLoaded?.(file);
  }, [file, onLoaded]);

  const goTo = useCallback(
    (step: 1 | -1) => {
      const target = hunkTarget(built.hunkStarts, firstRow.current, step);
      if (target == null) return;
      listRef.current?.scrollToIndex({ index: target, animated: false, viewPosition: 0.1 });
      firstRow.current = target;
    },
    [built.hunkStarts],
  );
  useEffect(() => registerDiffController({ nextHunk: () => goTo(1), prevHunk: () => goTo(-1) }), [goTo]);

  const onViewable = useCallback(({ viewableItems }: { viewableItems: { index: number | null }[] }) => {
    const first = viewableItems[0]?.index;
    if (first != null) firstRow.current = first;
  }, []);

  const renderItem = useCallback(
    ({ item }: { item: DiffRow }) => {
      if (item.kind === "hunk") {
        return <HunkRow header={item.header} actions={renderHunkActions?.(item.hunk, item.header) ?? null} />;
      }
      if (item.kind === "line") return <LineRow cell={item.cell} language={language} unified onSelect={onSelectLine} />;
      return <PairRow left={item.left} right={item.right} language={language} onSelect={onSelectLine} />;
    },
    [language, onSelectLine, renderHunkActions],
  );

  const getItemLayout = useCallback(
    (_: ArrayLike<DiffRow> | null | undefined, index: number) => {
      // Hunk headers are taller, so offsets are the sum of what precedes: precomputed once per file.
      return { length: heightOf(built.rows[index]), offset: built.offsets[index] ?? 0, index };
    },
    [built],
  );

  if (loading && !file) return <DiffSkeleton />;
  if (error && !file) {
    return <EmptyState icon={FileDiff} message={`Could not load this diff. ${error.message}`} />;
  }
  if (!file) return <EmptyState icon={FileDiff} message="This file has no changes any more." />;
  if (file.binary) return <EmptyState icon={FileDiff} message="Binary file changed. There is no text diff to show." />;
  if (built.rows.length === 0) return <EmptyState icon={FileDiff} message="The change is only to permissions or line endings." />;

  return (
    <View style={styles.fill}>
      {file.skipped_lines > 0 ? <SkippedNote count={file.skipped_lines} /> : null}
      <BoundedList<DiffRow>
        ref={listRef}
        data={built.rows}
        keyExtractor={rowKey}
        renderItem={({ item }) => renderItem({ item })}
        getItemLayout={getItemLayout}
        initialNumToRender={40}
        maxToRenderPerBatch={40}
        windowSize={11}
        onViewableItemsChanged={onViewable as never}
        viewabilityConfig={VIEWABILITY}
        onScrollToIndexFailed={(info) => listRef.current?.scrollToOffset({ offset: info.averageItemLength * info.index, animated: false })}
        ListEmptyComponent={<View />}
      />
    </View>
  );
}

const VIEWABILITY = { itemVisiblePercentThreshold: 50 };
const rowKey = (row: DiffRow) => row.key;
const heightOf = (row: DiffRow | undefined) => (row?.kind === "hunk" ? HUNK_HEIGHT : DIFF_LINE_HEIGHT);

function SkippedNote({ count }: { count: number }) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.note, { borderBottomColor: tokens.border }]}>
      <Text style={[styles.noteText, { color: tokens.ink3 }]}>
        {count} more changed {count === 1 ? "line is" : "lines are"} not shown. Open the file to read all of it.
      </Text>
    </View>
  );
}

function DiffSkeleton() {
  return (
    <View style={styles.skeleton}>
      {Array.from({ length: 10 }, (_, i) => (
        <Skeleton key={i} width={`${40 + ((i * 17) % 50)}%`} height={10} />
      ))}
    </View>
  );
}

const HunkRow = React.memo(function HunkRow({ header, actions }: { header: string; actions: React.ReactNode }) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.hunk, { backgroundColor: tokens.bg1, borderTopColor: tokens.border, borderBottomColor: tokens.border }]}>
      <Text numberOfLines={1} style={[styles.hunkText, { color: tokens.ink3 }]}>
        {header}
      </Text>
      {actions}
    </View>
  );
});

function fillFor(kind: DiffCell["kind"], tokens: ColorTokens): { fill: string; gutter: string } {
  if (kind === "add") return { fill: hexToRgba(tokens.success, 0.1), gutter: tokens.success };
  if (kind === "del") return { fill: hexToRgba(tokens.danger, 0.09), gutter: tokens.danger };
  return { fill: "transparent", gutter: tokens.ink4 };
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

const Code = React.memo(function Code({ cell, language }: { cell: DiffCell; language: string | null }) {
  const { tokens, scheme } = useTheme();
  const keyword = scheme === "dark" ? tokens.ember.ember300 : tokens.ember.ember600;
  const spans = useMemo(() => styledSpans(cell.text, language, cell.segments), [cell, language]);
  const changedFill = cell.kind === "add" ? hexToRgba(tokens.success, 0.26) : hexToRgba(tokens.danger, 0.24);
  return (
    <Text numberOfLines={1} style={[styles.code, { color: cell.kind === "context" ? tokens.ink2 : tokens.ink }]}>
      {cell.text.length === 0
        ? " "
        : spans.map((span, index) => (
            <Text key={index} style={[tokenColor(span.kind, tokens, keyword), span.changed ? { backgroundColor: changedFill } : null]}>
              {span.text}
            </Text>
          ))}
    </Text>
  );
});

const LineRow = React.memo(function LineRow({
  cell,
  language,
  onSelect,
}: {
  cell: DiffCell;
  language: string | null;
  unified: boolean;
  onSelect?: DiffViewProps["onSelectLine"];
}) {
  const { tokens } = useTheme();
  const { fill, gutter } = fillFor(cell.kind, tokens);
  const mark = cell.kind === "add" ? "+" : cell.kind === "del" ? "−" : " ";
  return (
    <View
      style={[styles.line, { backgroundColor: fill }]}
      // @ts-expect-error react-native-web pointer handler; PR C wires inline comments through it
      onClick={onSelect ? () => onSelect(cell.kind === "del" ? "old" : "new", cell) : undefined}
    >
      <Text style={[styles.lineNo, tabularNums, { color: gutter }]}>{cell.lineNo}</Text>
      <Text style={[styles.mark, { color: gutter }]}>{mark}</Text>
      <Code cell={cell} language={language} />
    </View>
  );
});

const Half = React.memo(function Half({ cell, language, last }: { cell: DiffCell | null; language: string | null; last?: boolean }) {
  const { tokens } = useTheme();
  if (!cell) return <View style={[styles.half, !last && { borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth }]} />;
  const { fill, gutter } = fillFor(cell.kind, tokens);
  return (
    <View
      style={[
        styles.half,
        { backgroundColor: fill },
        !last && { borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth },
      ]}
    >
      <Text style={[styles.lineNo, tabularNums, { color: gutter }]}>{cell.lineNo}</Text>
      <Code cell={cell} language={language} />
    </View>
  );
});

const PairRow = React.memo(function PairRow({
  left,
  right,
  language,
}: {
  left: DiffCell | null;
  right: DiffCell | null;
  language: string | null;
  onSelect?: DiffViewProps["onSelectLine"];
}) {
  return (
    <View style={styles.pair}>
      <Half cell={left} language={language} />
      <Half cell={right} language={language} last />
    </View>
  );
});

const styles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0 },
  skeleton: { padding: space.space16, gap: space.space12 },
  note: { paddingHorizontal: space.space16, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  noteText: { fontFamily: sansFamilyFor(400), fontSize: 11.5, lineHeight: 16 },
  hunk: {
    height: HUNK_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space8,
    paddingHorizontal: space.space16,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  hunkText: { flexShrink: 1, fontFamily: monoFamily.regular, fontSize: 10.5 },
  line: { height: DIFF_LINE_HEIGHT, flexDirection: "row", alignItems: "center", gap: space.space8, paddingRight: space.space16 },
  pair: { height: DIFF_LINE_HEIGHT, flexDirection: "row" },
  half: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: space.space8, paddingRight: space.space8 },
  lineNo: { width: 44, textAlign: "right", fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: DIFF_LINE_HEIGHT, opacity: 0.8 },
  mark: { width: 8, fontFamily: monoFamily.regular, fontSize: 11 },
  code: { flex: 1, minWidth: 0, fontFamily: monoFamily.regular, fontSize: 11.5, lineHeight: DIFF_LINE_HEIGHT },
});
