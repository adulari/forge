// Machined git review dock — the main diff pane (docs/design/machined
// "Forge Machined - Desktop.dc.html" L265-308): a header carrying the path, its counts and the
// split/unified toggle, then the file's hunks. Split = old|new columns with per-side line
// numbers; unified = one column. Changed lines are tinted with the design's exact alphas
// (add .09 success, del .08 danger) via `hexToRgba`, never a raw literal.
//
// The pane renders whatever `GET /api/git/diff` returned and nothing else: a binary file gets
// the daemon's one-line fact and no hunks, a rename shows both names, and a diff the daemon
// capped reports the omitted-line count rather than ending mid-file with no explanation.
import { FileDiff } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { type DiffCell, middleTruncate, type SplitRow, toSplitRows, toUnifiedRows } from "./diffModel";
import { type GitDiffFile } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { type ColorTokens, hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";

export type DiffViewMode = "split" | "unified";

export interface GitDiffPaneProps {
  /** The single file `GET /api/git/diff?path=…` returned, or null when the request came back
   * with no files (path unchanged in the requested index/worktree). */
  file: GitDiffFile | null;
  /** Bucket the selection came from — decides the "in the index" / "in the working tree"
   * wording of the no-diff case. */
  staged: boolean;
  hasSelection: boolean;
  loading: boolean;
  error: Error | null;
}

const LINE_HEIGHT = 19;
const LINE_NO_WIDTH = 30;
const MONO_ADVANCE = 0.6;
const HEADER_FONT_SIZE = 11.5;
/** Chevron + counts + kind + the split/unified toggle. */
const HEADER_CHROME_PX = 240;
/** Below this the two columns are narrower than a typical code line, so split is not offered. */
const SPLIT_MIN_WIDTH = 560;

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

function DiffLine({ cell, gutterChar }: { cell: DiffCell | null; gutterChar?: boolean }) {
  const tokens = useTokens();
  if (!cell) {
    // The empty side of an unpaired change: a spacer, never a fabricated blank code line.
    return <View style={styles.line} />;
  }
  const { fill, ink, gutterInk } = cellColors(cell.kind, tokens);
  const prefix = cell.kind === "add" ? "+" : cell.kind === "del" ? "−" : " ";
  return (
    <View style={[styles.line, { backgroundColor: fill }]}>
      <Text style={[styles.lineNo, tabularNums, { color: gutterInk }]}>{cell.lineNo}</Text>
      {gutterChar ? <Text style={[styles.gutterChar, { color: gutterInk }]}>{prefix}</Text> : null}
      <Text selectable style={[styles.lineText, { color: ink }]}>
        {cell.text.length > 0 ? cell.text : " "}
      </Text>
    </View>
  );
}

function HunkHeader({ header }: { header: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.hunkHeader, { borderTopColor: tokens.border }]}>
      <Text selectable style={[styles.lineText, { color: tokens.info }]} numberOfLines={1}>
        {header}
      </Text>
    </View>
  );
}

function SplitBody({ hunkRows }: { hunkRows: SplitRow[] }) {
  const tokens = useTokens();
  return (
    <>
      {toBlocks(hunkRows).map((block) => (
        <View key={block.key}>
          <HunkHeader header={block.header} />
          <View style={styles.splitRow}>
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              style={[styles.column, { borderRightColor: tokens.border }]}
            >
              <View style={styles.columnBody}>
                {block.pairs.map((pair) => (
                  <DiffLine key={`l:${pair.key}`} cell={pair.left} />
                ))}
              </View>
            </ScrollView>
            <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.columnLast}>
              <View style={styles.columnBody}>
                {block.pairs.map((pair) => (
                  <DiffLine key={`r:${pair.key}`} cell={pair.right} />
                ))}
              </View>
            </ScrollView>
          </View>
        </View>
      ))}
    </>
  );
}

function ModeToggle({
  mode,
  onChange,
  splitAvailable,
}: {
  mode: DiffViewMode;
  onChange: (mode: DiffViewMode) => void;
  splitAvailable: boolean;
}) {
  const tokens = useTokens();
  const options: DiffViewMode[] = ["split", "unified"];
  return (
    <View style={styles.toggle}>
      {options.map((option) => {
        const active = option === mode;
        const disabled = option === "split" && !splitAvailable;
        return (
          <Pressable
            key={option}
            onPress={disabled ? undefined : () => onChange(option)}
            disabled={disabled}
            accessibilityRole="tab"
            accessibilityState={{ selected: active, disabled }}
            accessibilityLabel={`${option} diff`}
            accessibilityHint={disabled ? "the pane is too narrow for two columns" : undefined}
            style={[
              styles.toggleChip,
              {
                borderRadius: radii.radiusSegmentInner,
                borderColor: active ? tokens.border : "transparent",
                backgroundColor: active ? tokens.bg3 : "transparent",
              },
            ]}
          >
            <Text
              style={[
                typeScale.monoMeta,
                { color: disabled ? tokens.ink4 : active ? tokens.ink2 : tokens.ink3 },
              ]}
            >
              {option}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

export function GitDiffPane({ file, staged, hasSelection, loading, error }: GitDiffPaneProps) {
  const tokens = useTokens();
  const [mode, setMode] = useState<DiffViewMode>("split");
  const [width, setWidth] = useState(0);

  const splitAvailable = width === 0 || width >= SPLIT_MIN_WIDTH;
  const effectiveMode: DiffViewMode = splitAvailable ? mode : "unified";
  const pathBudget = Math.max(
    12,
    Math.floor((Math.max(width, 320) - HEADER_CHROME_PX) / (HEADER_FONT_SIZE * MONO_ADVANCE)),
  );

  const headerPath = file
    ? file.orig_path
      ? `${middleTruncate(file.orig_path, Math.floor(pathBudget / 2))} → ${middleTruncate(file.path, Math.floor(pathBudget / 2))}`
      : middleTruncate(file.path, pathBudget)
    : "";

  let body: React.ReactNode;
  if (!hasSelection) {
    body = <EmptyState icon={FileDiff} message="Select a file to review its diff." />;
  } else if (loading) {
    body = <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>Loading diff…</Text>;
  } else if (error) {
    body = <Text style={[typeScale.sub, styles.notice, { color: tokens.danger }]}>{error.message}</Text>;
  } else if (!file) {
    body = (
      <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>
        No diff for this path {staged ? "in the index" : "in the working tree"}.
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
        {file.kind} with no content change (mode or metadata only).
      </Text>
    );
  } else {
    body = (
      <ScrollView style={styles.scroll} contentContainerStyle={styles.scrollBody}>
        {effectiveMode === "split" ? (
          <SplitBody hunkRows={toSplitRows(file.hunks)} />
        ) : (
          <ScrollView horizontal showsHorizontalScrollIndicator={false}>
            <View style={styles.columnBody}>
              {toUnifiedRows(file.hunks).map((row) =>
                row.kind === "hunk" ? (
                  <HunkHeader key={row.key} header={row.header} />
                ) : (
                  <DiffLine key={row.key} cell={row.cell} gutterChar />
                ),
              )}
            </View>
          </ScrollView>
        )}
        {file.skipped_lines > 0 ? (
          <Text style={[typeScale.monoMeta, styles.notice, { color: tokens.ink4 }]}>
            {file.skipped_lines} more line{file.skipped_lines === 1 ? "" : "s"} omitted — the daemon
            caps a single file&apos;s diff.
          </Text>
        ) : null}
      </ScrollView>
    );
  }

  return (
    <View style={styles.pane} onLayout={(event) => setWidth(event.nativeEvent.layout.width)}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        {file ? (
          <>
            <Text style={[styles.headerPath, { color: tokens.ink }]} numberOfLines={1}>
              {headerPath}
            </Text>
            <Text style={[styles.headerMeta, { color: tokens.ink3 }]}>{file.kind}</Text>
            {file.binary ? (
              <Text style={[styles.headerMeta, { color: tokens.ink3 }]}>bin</Text>
            ) : (
              <>
                <Text style={[styles.headerMeta, tabularNums, { color: tokens.success }]}>+{file.adds}</Text>
                <Text style={[styles.headerMeta, tabularNums, { color: tokens.danger }]}>−{file.dels}</Text>
              </>
            )}
          </>
        ) : (
          <Text style={[styles.headerPath, { color: tokens.ink3 }]} numberOfLines={1}>
            {hasSelection ? "…" : "no file selected"}
          </Text>
        )}
        <View style={styles.headerSpacer} />
        <ModeToggle mode={effectiveMode} onChange={setMode} splitAvailable={splitAvailable} />
      </View>
      {body}
    </View>
  );
}

const styles = StyleSheet.create({
  pane: { flex: 1, minWidth: 0 },
  header: {
    height: 34,
    flexShrink: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
    paddingHorizontal: space.space16,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  headerPath: { fontSize: HEADER_FONT_SIZE, lineHeight: 16, fontFamily: monoFamily.regular, flexShrink: 1 },
  headerMeta: { fontSize: 9.5, lineHeight: 13, fontFamily: monoFamily.regular },
  headerSpacer: { flex: 1 },
  toggle: { flexDirection: "row", gap: space.space4 },
  toggleChip: { paddingHorizontal: 6, paddingVertical: 1, borderWidth: 1 },
  scroll: { flex: 1 },
  scrollBody: { paddingBottom: space.space12 },
  splitRow: { flexDirection: "row" },
  column: { flex: 1, borderRightWidth: StyleSheet.hairlineWidth },
  columnLast: { flex: 1 },
  columnBody: { minWidth: "100%", paddingVertical: space.space4 },
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
    justifyContent: "center",
    paddingHorizontal: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  notice: { paddingHorizontal: space.space16, paddingVertical: space.space12 },
});
