// One file's diff, virtualized. Every row is a fixed height so a 5,000-line diff costs a list of
// numbers, and only the rows near the viewport mount. Built on diffModel/diffHighlight (the same
// pairing and syntax rules the Review dock uses).
//
// Each hunk header carries Accept (stage), Revert (with an undo toast), Comment and Explain; a click
// on a line comments on it. The keys a / x / c / ? / n / p act on the hunk in view. Row components
// stay memoised on primitives: the handlers live in refs so a mutation never re-renders 5,000 rows.
import { Check, FileDiff, HelpCircle, MessageSquare, Undo2 } from "lucide-react-native";
import React, { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type FlatList, Pressable, StyleSheet, Text, View } from "react-native";

import type { BlameLine, GitDiffFile, GitHunkAction } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useGitHunk } from "../../lib/queries";
import { reviewDiffRevision, useReviewComments } from "../../lib/reviewComments";
import { type TokenKind } from "../../lib/highlightTokens";
import { useTheme } from "../../theme/ThemeProvider";
import { type ColorTokens, hexToRgba, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor, tabularNums } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { useToast } from "../ds/ToastHost";
import type { DiffCell } from "../git/diffModel";
import { languageForPath, styledSpans } from "../git/diffHighlight";
import { buildHunkPatch, classifyHunkError, hunkActionDone, hunkActionsFor } from "../git/hunkActions";
import { isGitReviewReadOnly } from "../git/gitReviewSupport";
import { type BlameMap, isTraceable } from "./blameModel";
import { registerDiffController } from "./diffController";
import { useFileDiff } from "./useFileDiff";
import { DIFF_LINE_HEIGHT, HUNK_HEIGHT, buildDiffRows, hunkTarget, type DiffMode, type DiffRow } from "./diffRows";
import { currentHunkIndex, hunkSelection, lineSelection } from "./hunkSelection";
import { undoRequests, undoStepForHunk, type UndoStep } from "./hunkUndo";
import { useWhyTraceOn } from "./ideSignals";
import { IdeTextButton } from "./ideUi";
import { CommentComposer, useAgentSend, type CommentTarget } from "./ReviewDraft";
import { explainPrompt } from "./reviewDraft";

const WhyTraceLoader = React.lazy(() => import("./WhyTraceGutter").then((m) => ({ default: m.WhyTraceLoader })));
const WhyTraceCard = React.lazy(() => import("./WhyTraceGutter").then((m) => ({ default: m.WhyTraceCard })));

const SPLIT_BREAKPOINT = 900;
const COMPACT_ACTIONS_BELOW = 640;
const UNDO_TOAST_MS = 9000;
const EMPTY_MAP: BlameMap = new Map();
const clickProps = (handler?: () => void) => (handler ? ({ onClick: handler } as object) : {});

export interface DiffViewProps {
  sessionId: string;
  path: string;
  /** The session owns a worktree: its diff is a whole-branch diff rather than a git index diff. */
  worktree: boolean;
  mode?: DiffMode;
  onLoaded?: (file: GitDiffFile | null) => void;
}

type SelectLine = (side: "old" | "new", cell: DiffCell) => void;
type TraceLine = (line: number, info: BlameLine) => void;

export function DiffView({ sessionId, path, worktree, mode: forcedMode, onLoaded }: DiffViewProps) {
  const { file, loading, error, bucket } = useFileDiff(sessionId, path, worktree);
  const [paneWidth, setPaneWidth] = useState(0);
  const mode: DiffMode = forcedMode ?? (paneWidth >= SPLIT_BREAKPOINT ? "split" : "unified");
  const language = useMemo(() => languageForPath(path), [path]);
  const built = useMemo(() => buildDiffRows(file?.hunks ?? [], mode), [file, mode]);
  const listRef = useRef<FlatList<DiffRow>>(null);
  const firstRow = useRef(0);
  const [activeHunk, setActiveHunk] = useState(0);
  const toast = useToast();
  const { baseUrl } = useAuth();
  const hunkMutation = useGitHunk();
  const send = useAgentSend();
  const traceOn = useWhyTraceOn();
  const [trace, setTrace] = useState<BlameMap>(EMPTY_MAP);
  const [traceCard, setTraceCard] = useState<{ line: number; info: BlameLine } | null>(null);
  const [target, setTarget] = useState<CommentTarget | null>(null);
  const comments = useReviewComments(sessionId);
  const readOnly = isGitReviewReadOnly(baseUrl);

  useEffect(() => {
    onLoaded?.(file);
  }, [file, onLoaded]);

  const revision = useMemo(() => (file ? reviewDiffRevision(file.path, file.hunks) : ""), [file]);
  const specs = useMemo(() => {
    if (!file) return [];
    const all = hunkActionsFor(bucket, file, readOnly);
    // A worktree session's diff is the whole branch: only the working-tree revert is meaningful.
    return worktree ? all.filter((spec) => spec.action === "revert") : all;
  }, [file, bucket, readOnly, worktree]);
  const canAccept = specs.some((spec) => spec.action === "stage" || spec.action === "unstage");
  const canRevert = specs.some((spec) => spec.action === "revert");

  const live = useRef({ file, bucket, worktree, revision, hunkMutation, toast, send, sessionId, path, hunkStarts: built.hunkStarts, activeHunk });
  useEffect(() => {
    live.current = { file, bucket, worktree, revision, hunkMutation, toast, send, sessionId, path, hunkStarts: built.hunkStarts, activeHunk };
  });

  const undo = useCallback(async (steps: readonly UndoStep[]) => {
    const { hunkMutation: mutation, toast: toaster, sessionId: session, path: filePath } = live.current;
    try {
      for (const request of undoRequests(session, filePath, steps)) await mutation.mutateAsync(request);
      toaster.show("Restored", { tone: "success" });
    } catch (failure) {
      toaster.show(`Could not undo: ${classifyHunkError(failure).message}`, { tone: "danger" });
    }
  }, [live]);

  const runHunk = useCallback(
    async (index: number, action: GitHunkAction) => {
      const { file: current, hunkMutation: mutation, toast: toaster, sessionId: session, path: filePath } = live.current;
      const hunk = current?.hunks[index];
      if (!current || !hunk) return;
      try {
        await mutation.mutateAsync({ session, path: filePath, hunk_header: hunk.header, patch: buildHunkPatch(current, hunk), action });
        if (action === "revert") {
          const step = undoStepForHunk(current, hunk);
          toaster.show("Hunk reverted", { duration: UNDO_TOAST_MS, action: { label: "Undo", onPress: () => void undo([step]) } });
        } else {
          toaster.show(hunkActionDone(action), { tone: "success" });
        }
      } catch (failure) {
        const result = classifyHunkError(failure);
        toaster.show(result.message, { tone: result.kind === "error" ? "danger" : "neutral" });
      }
    },
    [live, undo],
  );

  const accept = useCallback(
    (index: number) => {
      const { bucket: where, worktree: isWorktree, toast: toaster } = live.current;
      if (isWorktree) {
        toaster.show("This agent works in a worktree, so there is nothing to stage. Press r to mark the file reviewed.", { tone: "neutral" });
        return;
      }
      if (where === "staged") {
        toaster.show("Already staged. Use Unstage in the Git tab to take it back.", { tone: "neutral" });
        return;
      }
      void runHunk(index, "stage");
    },
    [live, runHunk],
  );
  const revert = useCallback(
    (index: number) => {
      const { bucket: where, toast: toaster } = live.current;
      if (where === "staged") {
        toaster.show("Unstage this hunk first. Revert only changes the working tree.", { tone: "neutral" });
        return;
      }
      void runHunk(index, "revert");
    },
    [live, runHunk],
  );
  const commentHunk = useCallback((index: number) => {
    const { file: current, revision: rev } = live.current;
    const hunk = current?.hunks[index];
    const selection = hunk ? hunkSelection(hunk) : null;
    if (selection) setTarget({ selection, revision: rev });
  }, [live]);
  const explain = useCallback((index: number) => {
    const { file: current, send: sendPrompt, sessionId: session, path: filePath, toast: toaster } = live.current;
    const hunk = current?.hunks[index];
    if (!hunk) return;
    const where = sendPrompt(session, explainPrompt(filePath, hunk.header, hunk.lines));
    toaster.show(where === "sent" ? "Asked the agent. The answer appears in the agent panel." : "The question will send when the agent opens.", { tone: "neutral" });
  }, [live]);

  const goTo = useCallback(
    (step: 1 | -1) => {
      const target = hunkTarget(built.hunkStarts, firstRow.current, step);
      if (target == null) return;
      listRef.current?.scrollToIndex({ index: target, animated: false, viewPosition: 0.1 });
      firstRow.current = target;
      setActiveHunk(currentHunkIndex(built.hunkStarts, target));
    },
    [built.hunkStarts],
  );
  useEffect(
    () =>
      registerDiffController({
        nextHunk: () => goTo(1),
        prevHunk: () => goTo(-1),
        acceptHunk: () => accept(live.current.activeHunk),
        revertHunk: () => revert(live.current.activeHunk),
        commentHunk: () => commentHunk(live.current.activeHunk),
        explainHunk: () => explain(live.current.activeHunk),
      }),
    [live, goTo, accept, revert, commentHunk, explain],
  );

  const onViewable = useCallback(({ viewableItems }: { viewableItems: { index: number | null }[] }) => {
    const first = viewableItems[0]?.index;
    if (first == null) return;
    firstRow.current = first;
    setActiveHunk(currentHunkIndex(live.current.hunkStarts, first));
  }, [live]);

  const onSelectLine = useCallback<SelectLine>((_side, cell) => {
    setTarget({ selection: lineSelection(cell), revision: live.current.revision });
    setTraceCard(null);
  }, [live]);
  const onTrace = useCallback<TraceLine>((line, info) => setTraceCard({ line, info }), []);
  const onMap = useCallback((map: BlameMap) => setTrace(map), []);

  const commented = useMemo(() => {
    const set = new Set<string>();
    for (const comment of comments) {
      if (comment.path !== path) continue;
      for (let line = comment.startLine; line <= comment.endLine; line += 1) set.add(`${comment.side}:${line}`);
    }
    return set;
  }, [comments, path]);

  const compact = paneWidth > 0 && paneWidth < COMPACT_ACTIONS_BELOW;
  const renderItem = useCallback(
    ({ item }: { item: DiffRow }) => {
      if (item.kind === "hunk") {
        return (
          <HunkRow
            header={item.header}
            active={item.hunk === activeHunk}
            index={item.hunk}
            staged={bucket === "staged"}
            canAccept={canAccept}
            canRevert={canRevert}
            compact={compact}
            onAccept={accept}
            onRevert={revert}
            onComment={commentHunk}
            onExplain={explain}
          />
        );
      }
      const showTrace = traceOn;
      if (item.kind === "line") {
        const side = item.cell.kind === "del" ? "old" : "new";
        const info = showTrace && isTraceable(side, item.cell.kind) ? trace.get(item.cell.lineNo) : undefined;
        return (
          <LineRow
            cell={item.cell}
            language={language}
            onSelect={onSelectLine}
            commented={commented.has(`${side}:${item.cell.lineNo}`)}
            gutter={showTrace}
            trace={info}
            onTrace={onTrace}
          />
        );
      }
      const info = showTrace && item.right && isTraceable("new", item.right.kind) ? trace.get(item.right.lineNo) : undefined;
      return (
        <PairRow
          left={item.left}
          right={item.right}
          language={language}
          onSelect={onSelectLine}
          gutter={showTrace}
          trace={info}
          onTrace={onTrace}
          commentedLeft={item.left ? commented.has(`old:${item.left.lineNo}`) : false}
          commentedRight={item.right ? commented.has(`new:${item.right.lineNo}`) : false}
        />
      );
    },
    [activeHunk, bucket, canAccept, canRevert, compact, accept, revert, commentHunk, explain, language, onSelectLine, commented, traceOn, trace, onTrace],
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
    <View style={styles.fill} onLayout={(event) => setPaneWidth(Math.round(event.nativeEvent.layout.width))}>
      {file.skipped_lines > 0 ? <SkippedNote count={file.skipped_lines} /> : null}
      <BoundedList<DiffRow>
        ref={listRef}
        data={built.rows}
        extraData={renderItem}
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
      {traceOn ? (
        <Suspense fallback={null}>
          <WhyTraceLoader sessionId={sessionId} path={path} onMap={onMap} />
          {traceCard ? <WhyTraceCard info={traceCard.info} path={path} line={traceCard.line} onClose={() => setTraceCard(null)} /> : null}
        </Suspense>
      ) : null}
      {target ? (
        <CommentComposer key={`${target.selection.side}:${target.selection.startLine}`} sessionId={sessionId} path={path} staged={bucket === "staged"} target={target} onDone={() => setTarget(null)} />
      ) : null}
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

const HunkRow = React.memo(function HunkRow({
  header,
  active,
  index,
  staged,
  canAccept,
  canRevert,
  compact,
  onAccept,
  onRevert,
  onComment,
  onExplain,
}: {
  header: string;
  active: boolean;
  index: number;
  staged: boolean;
  canAccept: boolean;
  canRevert: boolean;
  compact: boolean;
  onAccept: (index: number) => void;
  onRevert: (index: number) => void;
  onComment: (index: number) => void;
  onExplain: (index: number) => void;
}) {
  const { tokens } = useTheme();
  return (
    <View style={[styles.hunk, { backgroundColor: active ? tokens.bg2 : tokens.bg1, borderTopColor: tokens.border, borderBottomColor: tokens.border }]}>
      <Text numberOfLines={1} style={[styles.hunkText, { color: active ? tokens.ink2 : tokens.ink3 }]}>
        {header}
      </Text>
      <View style={styles.hunkActions}>
        {canAccept ? <HunkButton label={staged ? "Staged" : "Accept"} keyHint="a" compact={compact} icon={<Check size={12} strokeWidth={2} color={tokens.ink2} />} onPress={() => onAccept(index)} disabled={staged} /> : null}
        {canRevert ? <HunkButton label="Revert" keyHint="x" compact={compact} icon={<Undo2 size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => onRevert(index)} /> : null}
        <HunkButton label="Comment" keyHint="c" compact={compact} icon={<MessageSquare size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => onComment(index)} />
        <HunkButton label="Explain" keyHint="?" compact icon={<HelpCircle size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => onExplain(index)} />
      </View>
    </View>
  );
});

function HunkButton({ label, keyHint, compact, icon, onPress, disabled }: { label: string; keyHint: string; compact: boolean; icon: React.ReactNode; onPress: () => void; disabled?: boolean }) {
  // Narrow panes keep the icon and drop the words; the tooltip and the key still name the action.
  if (compact) {
    return (
      <Pressable
        onPress={disabled ? undefined : onPress}
        accessibilityRole="button"
        accessibilityLabel={label}
        // @ts-expect-error react-native-web tooltip
        title={`${label} (${keyHint})`}
        style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.hunkIcon, { opacity: disabled ? 0.45 : 1 }, hovered ? hoverStyle : null]}
      >
        {icon}
      </Pressable>
    );
  }
  return <IdeTextButton label={label} icon={icon} keyHint={keyHint} onPress={onPress} disabled={disabled} />;
}
const hoverStyle = { backgroundColor: "rgba(127,127,127,0.18)" };

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

function TraceBar({ line, info, onTrace }: { line: number; info?: BlameLine; onTrace: TraceLine }) {
  const { tokens } = useTheme();
  if (!info) return <View style={styles.traceSlot} />;
  return (
    <Pressable
      onPress={() => onTrace(line, info)}
      accessibilityRole="button"
      accessibilityLabel={`Why was line ${line} written`}
      // @ts-expect-error react-native-web tooltip
      title={`Turn ${info.turn}${info.prompt_summary ? `: ${info.prompt_summary}` : ""}`}
      style={styles.traceSlot}
    >
      <View style={[styles.traceBar, { backgroundColor: tokens.ink3 }]} />
    </Pressable>
  );
}

const LineRow = React.memo(function LineRow({
  cell,
  language,
  onSelect,
  commented,
  gutter,
  trace,
  onTrace,
}: {
  cell: DiffCell;
  language: string | null;
  onSelect: SelectLine;
  commented: boolean;
  gutter: boolean;
  trace?: BlameLine;
  onTrace: TraceLine;
}) {
  const { tokens } = useTheme();
  const { fill, gutter: edge } = fillFor(cell.kind, tokens);
  const mark = cell.kind === "add" ? "+" : cell.kind === "del" ? "−" : " ";
  return (
    <View style={[styles.line, { backgroundColor: fill }]} {...clickProps(() => onSelect(cell.kind === "del" ? "old" : "new", cell))}>
      {gutter ? <TraceBar line={cell.lineNo} info={trace} onTrace={onTrace} /> : null}
      <Text style={[styles.lineNo, tabularNums, { color: edge }]}>{cell.lineNo}</Text>
      <Text style={[styles.mark, { color: edge }]}>{mark}</Text>
      <Code cell={cell} language={language} />
      {commented ? <MessageSquare size={10} strokeWidth={2} color={tokens.ink2} /> : null}
    </View>
  );
});

const Half = React.memo(function Half({
  cell,
  language,
  last,
  side,
  onSelect,
  commented,
  gutter,
  trace,
  onTrace,
}: {
  cell: DiffCell | null;
  language: string | null;
  last?: boolean;
  side: "old" | "new";
  onSelect: SelectLine;
  commented: boolean;
  gutter: boolean;
  trace?: BlameLine;
  onTrace: TraceLine;
}) {
  const { tokens } = useTheme();
  if (!cell) return <View style={[styles.half, !last && { borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth }]} />;
  const { fill, gutter: edge } = fillFor(cell.kind, tokens);
  return (
    <View
      style={[
        styles.half,
        { backgroundColor: fill },
        !last && { borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth },
      ]}
      {...clickProps(() => onSelect(side, cell))}
    >
      {gutter && last ? <TraceBar line={cell.lineNo} info={trace} onTrace={onTrace} /> : null}
      <Text style={[styles.lineNo, tabularNums, { color: edge }]}>{cell.lineNo}</Text>
      <Code cell={cell} language={language} />
      {commented ? <MessageSquare size={10} strokeWidth={2} color={tokens.ink2} /> : null}
    </View>
  );
});

const PairRow = React.memo(function PairRow({
  left,
  right,
  language,
  onSelect,
  gutter,
  trace,
  onTrace,
  commentedLeft,
  commentedRight,
}: {
  left: DiffCell | null;
  right: DiffCell | null;
  language: string | null;
  onSelect: SelectLine;
  gutter: boolean;
  trace?: BlameLine;
  onTrace: TraceLine;
  commentedLeft: boolean;
  commentedRight: boolean;
}) {
  return (
    <View style={styles.pair}>
      <Half cell={left} language={language} side="old" onSelect={onSelect} commented={commentedLeft} gutter={false} onTrace={onTrace} />
      <Half cell={right} language={language} last side="new" onSelect={onSelect} commented={commentedRight} gutter={gutter} trace={trace} onTrace={onTrace} />
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
  hunkActions: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  hunkIcon: { width: 24, height: 22, borderRadius: 4, alignItems: "center", justifyContent: "center" },
  traceSlot: { width: 8, height: DIFF_LINE_HEIGHT, alignItems: "center", justifyContent: "center" },
  traceBar: { width: 2, height: DIFF_LINE_HEIGHT - 4, borderRadius: 1, opacity: 0.7 },
  hunkText: { flexShrink: 1, fontFamily: monoFamily.regular, fontSize: 10.5 },
  line: { height: DIFF_LINE_HEIGHT, flexDirection: "row", alignItems: "center", gap: space.space8, paddingRight: space.space16 },
  pair: { height: DIFF_LINE_HEIGHT, flexDirection: "row" },
  half: { flex: 1, minWidth: 0, flexDirection: "row", alignItems: "center", gap: space.space8, paddingRight: space.space8 },
  lineNo: { width: 44, textAlign: "right", fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: DIFF_LINE_HEIGHT, opacity: 0.8 },
  mark: { width: 8, fontFamily: monoFamily.regular, fontSize: 11 },
  code: { flex: 1, minWidth: 0, fontFamily: monoFamily.regular, fontSize: 11.5, lineHeight: DIFF_LINE_HEIGHT },
});
