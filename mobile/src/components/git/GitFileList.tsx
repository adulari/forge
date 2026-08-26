// Machined git review dock — the ~264px file column (docs/design/machined
// "Forge Machined - Desktop.dc.html" L265-308): STAGED / UNSTAGED / UNTRACKED groups, one
// dense row per path with its porcelain status letter, a middle-truncated mono path, and
// +adds/−dels in success/danger. Every row comes from `GET /api/git/status`; nothing here
// is synthesised.
import { Minus, Plus } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { middleTruncate } from "./diffModel";
import { type GitFileRow, type GitStatusResponse } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { type ColorTokens, hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

/** A file row plus the bucket it was clicked in — `GET /api/git/diff` needs `staged` to know
 * whether to read the index or the working tree, and a file edited after staging legitimately
 * appears in BOTH buckets with different content. */
export interface GitSelection {
  path: string;
  staged: boolean;
}

export interface GitFileListProps {
  status: GitStatusResponse;
  selected: GitSelection | null;
  onSelect: (selection: GitSelection) => void;
  /** `null` over Anywhere, where the host refuses index mutations — rows render without actions. */
  onStage: ((paths: string[]) => void) | null;
  onUnstage: ((paths: string[]) => void) | null;
  /** An index mutation is in flight — row actions stay visible but stop accepting presses. */
  busy: boolean;
}

const ROW_HEIGHT = 28;
// Geist Mono advances ~0.6em; the path budget is derived from the measured column instead of
// a fixed character count so the dock survives a resized or stacked container.
const MONO_ADVANCE = 0.6;
const PATH_FONT_SIZE = 11.5;
/** Status letter + gaps + counts + the stage button — the width a path cannot use. */
const ROW_CHROME_PX = 122;

function statusColor(letter: string, tokens: ColorTokens): string {
  switch (letter) {
    case "A":
    case "?":
      return tokens.success;
    case "D":
      return tokens.danger;
    case "U":
      return tokens.danger;
    case "R":
    case "C":
      return tokens.info;
    default:
      // M, T, and anything git adds later.
      return tokens.warn;
  }
}

function FileRow({
  row,
  staged,
  selected,
  charBudget,
  busy,
  onSelect,
  onToggleIndex,
}: {
  row: GitFileRow;
  staged: boolean;
  selected: boolean;
  charBudget: number;
  busy: boolean;
  onSelect: () => void;
  /** `null` in read-only mode — the stage/unstage affordance is omitted rather than disabled. */
  onToggleIndex: (() => void) | null;
}) {
  const tokens = useTokens();
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);

  const renamedFrom = row.orig_path;
  const label = renamedFrom ? `${row.path}, renamed from ${renamedFrom}` : row.path;
  const background = selected ? hexToRgba(tokens.ink, 0.07) : hovered ? tokens.bg3 : "transparent";

  return (
    <View style={styles.rowWrap}>
      <Pressable
        onPress={onSelect}
        onHoverIn={() => setHovered(true)}
        onHoverOut={() => setHovered(false)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        accessibilityRole="button"
        accessibilityState={{ selected }}
        accessibilityLabel={label}
        style={[
          styles.row,
          {
            backgroundColor: background,
            borderRadius: radii.radius4,
            borderColor: focused ? tokens.accent : "transparent",
          },
        ]}
      >
        <Text style={[styles.statusLetter, { color: statusColor(row.status, tokens) }]}>{row.status}</Text>
        <Text
          style={[styles.path, { color: selected ? tokens.ink : tokens.ink2 }]}
          numberOfLines={1}
        >
          {middleTruncate(row.path, charBudget)}
        </Text>
        {row.binary ? (
          <Text style={[styles.count, { color: tokens.ink3 }]}>bin</Text>
        ) : (
          <>
            {row.adds > 0 ? (
              <Text style={[styles.count, tabularNums, { color: tokens.success }]}>+{row.adds}</Text>
            ) : null}
            {row.dels > 0 ? (
              <Text style={[styles.count, tabularNums, { color: tokens.danger }]}>−{row.dels}</Text>
            ) : null}
          </>
        )}
      </Pressable>
      {onToggleIndex ? (
        <Pressable
          onPress={busy ? undefined : onToggleIndex}
          disabled={busy}
          accessibilityRole="button"
          accessibilityLabel={`${staged ? "unstage" : "stage"} ${row.path}`}
          hitSlop={6}
          style={styles.rowAction}
        >
          {staged ? (
            <Minus size={13} strokeWidth={1.75} color={busy ? tokens.ink4 : tokens.ink3} />
          ) : (
            <Plus size={13} strokeWidth={1.75} color={busy ? tokens.ink4 : tokens.ink3} />
          )}
        </Pressable>
      ) : null}
    </View>
  );
}

function Group({
  title,
  rows,
  staged,
  actionLabel,
  selected,
  charBudget,
  busy,
  onSelect,
  onGroupAction,
  onRowAction,
}: {
  title: string;
  rows: GitFileRow[];
  staged: boolean;
  actionLabel: string;
  selected: GitSelection | null;
  charBudget: number;
  busy: boolean;
  onSelect: (selection: GitSelection) => void;
  /** `null` in read-only mode (Anywhere): the group/row index buttons are not rendered at all. */
  onGroupAction: ((paths: string[]) => void) | null;
  onRowAction: ((path: string) => void) | null;
}) {
  const tokens = useTokens();
  if (rows.length === 0) return null;

  return (
    <View style={styles.group}>
      <View style={styles.groupHeader}>
        {/* ds/SectionHeader carries 16px screen gutters — too wide for a 10px-gutter dock —
            so the label box is reproduced here at the dock's own density. */}
        <Text style={[typeScale.section, styles.groupLabel, { color: tokens.ink3 }]} numberOfLines={1}>
          {title} · {rows.length}
        </Text>
        {onGroupAction ? (
          <Pressable
            onPress={busy ? undefined : () => onGroupAction(rows.map((row) => row.path))}
            disabled={busy}
            accessibilityRole="button"
            accessibilityLabel={`${actionLabel} all ${rows.length} ${title.toLowerCase()} files`}
            hitSlop={6}
          >
            <Text style={[typeScale.monoMeta, { color: busy ? tokens.ink4 : tokens.ink3 }]}>{actionLabel} all</Text>
          </Pressable>
        ) : null}
      </View>
      {rows.map((row) => (
        <FileRow
          key={`${staged ? "s" : "w"}:${row.path}`}
          row={row}
          staged={staged}
          selected={selected?.path === row.path && selected.staged === staged}
          charBudget={charBudget}
          busy={busy}
          onSelect={() => onSelect({ path: row.path, staged })}
          onToggleIndex={onRowAction ? () => onRowAction(row.path) : null}
        />
      ))}
    </View>
  );
}

export function GitFileList({ status, selected, onSelect, onStage, onUnstage, busy }: GitFileListProps) {
  const tokens = useTokens();
  const [width, setWidth] = useState(0);
  const charBudget = Math.max(
    10,
    Math.floor((Math.max(width, 180) - ROW_CHROME_PX) / (PATH_FONT_SIZE * MONO_ADVANCE)),
  );

  return (
    <ScrollView
      style={styles.list}
      contentContainerStyle={styles.listContent}
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
    >
      <Group
        title="STAGED"
        rows={status.staged}
        staged
        actionLabel="unstage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        onSelect={onSelect}
        onGroupAction={onUnstage}
        onRowAction={onUnstage ? (path) => onUnstage([path]) : null}
      />
      <Group
        title="UNSTAGED"
        rows={status.unstaged}
        staged={false}
        actionLabel="stage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        onSelect={onSelect}
        onGroupAction={onStage}
        onRowAction={onStage ? (path) => onStage([path]) : null}
      />
      <Group
        title="UNTRACKED"
        rows={status.untracked}
        staged={false}
        actionLabel="stage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        onSelect={onSelect}
        onGroupAction={onStage}
        onRowAction={onStage ? (path) => onStage([path]) : null}
      />
      {status.truncated > 0 ? (
        <Text style={[typeScale.monoMeta, styles.truncated, { color: tokens.ink4 }]}>
          {status.truncated} more changed path{status.truncated === 1 ? "" : "s"} not listed — the
          daemon caps a status response
        </Text>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  list: { flex: 1 },
  listContent: { paddingHorizontal: 10, paddingVertical: space.space12 },
  group: { marginBottom: space.space8 },
  groupHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space8,
    paddingHorizontal: 6,
    paddingBottom: 6,
  },
  groupLabel: { fontFamily: monoFamily.regular, flexShrink: 1 },
  rowWrap: { flexDirection: "row", alignItems: "center", gap: 2 },
  row: {
    flex: 1,
    height: ROW_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
    paddingHorizontal: space.space8,
    borderWidth: 1,
  },
  statusLetter: { fontSize: 10, lineHeight: 14, fontFamily: monoFamily.bold, width: 9 },
  path: { flex: 1, fontSize: PATH_FONT_SIZE, lineHeight: 15, fontFamily: monoFamily.regular },
  count: { fontSize: 9.5, lineHeight: 13, fontFamily: monoFamily.regular },
  rowAction: { width: 20, height: ROW_HEIGHT, alignItems: "center", justifyContent: "center" },
  truncated: { paddingHorizontal: 6, paddingTop: space.space8 },
});
