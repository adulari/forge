// Machined git review dock — the ~264px file column (docs/design/machined
// "Forge Machined - Desktop.dc.html" L265-308): CONFLICTS / STAGED / UNSTAGED / UNTRACKED groups,
// one dense row per path with its porcelain status letter, a language-coloured file tag, a
// middle-truncated mono path, and +adds/−dels in success/danger. Every row comes from
// `GET /api/git/status`; nothing here is synthesised.
//
// Groups collapse (the header is the toggle). Unstaged and untracked rows can be discarded; the
// caller owns the confirmation, so a press here never destroys anything by itself. A conflicted
// path has no diff and no index action: the group offers "Resolve with Forge" instead.
import { ChevronDown, ChevronRight, Minus, Plus, Sparkles, Undo2 } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { middleTruncate } from "./diffModel";
import { type DiscardKind } from "./gitReviewSupport";
import { FileTypeIcon } from "./FileTypeIcon";
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
  /** Asks the caller to confirm, then discard working-tree changes. `null` when read-only. */
  onDiscard: ((paths: string[], kind: DiscardKind) => void) | null;
  /** Starts a turn that resolves the conflicted files. `null` when the dock cannot drive one. */
  onResolveConflicts: ((paths: string[]) => void) | null;
  /** An index mutation is in flight — row actions stay visible but stop accepting presses. */
  busy: boolean;
}

const ROW_HEIGHT = 28;
// Geist Mono advances ~0.6em; the path budget is derived from the measured column instead of
// a fixed character count so the dock survives a resized or stacked container.
const MONO_ADVANCE = 0.6;
const PATH_FONT_SIZE = 11.5;
/** Status letter + file tag + gaps + counts + two row actions — the width a path cannot use. */
const ROW_CHROME_PX = 150;

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

function RowAction({
  label,
  busy,
  onPress,
  children,
}: {
  label: string;
  busy: boolean;
  onPress: () => void;
  children: React.ReactNode;
}) {
  return (
    <Pressable
      onPress={busy ? undefined : onPress}
      disabled={busy}
      accessibilityRole="button"
      accessibilityLabel={label}
      hitSlop={4}
      style={styles.rowAction}
    >
      {children}
    </Pressable>
  );
}

function FileRow({
  row,
  staged,
  selected,
  charBudget,
  busy,
  onSelect,
  onToggleIndex,
  onDiscard,
}: {
  row: GitFileRow;
  staged: boolean;
  selected: boolean;
  charBudget: number;
  busy: boolean;
  onSelect: (() => void) | null;
  /** `null` in read-only mode — the stage/unstage affordance is omitted rather than disabled. */
  onToggleIndex: (() => void) | null;
  /** `null` for staged rows (unstage first) and in read-only mode. */
  onDiscard: (() => void) | null;
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
        onPress={onSelect ?? undefined}
        disabled={!onSelect}
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
        <FileTypeIcon path={row.path} />
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
      {onDiscard ? (
        <RowAction label={`discard changes to ${row.path}`} busy={busy} onPress={onDiscard}>
          <Undo2 size={12} strokeWidth={1.75} color={busy ? tokens.ink4 : tokens.ink3} />
        </RowAction>
      ) : null}
      {onToggleIndex ? (
        <RowAction label={`${staged ? "unstage" : "stage"} ${row.path}`} busy={busy} onPress={onToggleIndex}>
          {staged ? (
            <Minus size={13} strokeWidth={1.75} color={busy ? tokens.ink4 : tokens.ink3} />
          ) : (
            <Plus size={13} strokeWidth={1.75} color={busy ? tokens.ink4 : tokens.ink3} />
          )}
        </RowAction>
      ) : null}
    </View>
  );
}

function GroupHeaderButton({
  label,
  accessibilityLabel,
  busy,
  tone,
  onPress,
}: {
  label: string;
  accessibilityLabel: string;
  busy: boolean;
  tone?: "danger";
  onPress: () => void;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={busy ? undefined : onPress}
      disabled={busy}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      hitSlop={6}
    >
      <Text
        style={[
          typeScale.monoMeta,
          { color: busy ? tokens.ink4 : tone === "danger" ? tokens.danger : tokens.ink3 },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

function Group({
  id,
  title,
  rows,
  staged,
  actionLabel,
  selected,
  charBudget,
  busy,
  collapsed,
  onToggleCollapsed,
  onSelect,
  onGroupAction,
  onRowAction,
  discardKind,
  onDiscard,
}: {
  id: string;
  title: string;
  rows: GitFileRow[];
  staged: boolean;
  actionLabel: string;
  selected: GitSelection | null;
  charBudget: number;
  busy: boolean;
  collapsed: boolean;
  onToggleCollapsed: (id: string) => void;
  onSelect: (selection: GitSelection) => void;
  /** `null` in read-only mode (Anywhere): the group/row index buttons are not rendered at all. */
  onGroupAction: ((paths: string[]) => void) | null;
  onRowAction: ((path: string) => void) | null;
  discardKind: DiscardKind;
  onDiscard: ((paths: string[], kind: DiscardKind) => void) | null;
}) {
  const tokens = useTokens();
  if (rows.length === 0) return null;
  const paths = rows.map((row) => row.path);

  return (
    <View style={styles.group}>
      <View style={styles.groupHeader}>
        {/* ds/SectionHeader carries 16px screen gutters — too wide for a 10px-gutter dock —
            so the label box is reproduced here at the dock's own density. */}
        <Pressable
          onPress={() => onToggleCollapsed(id)}
          accessibilityRole="button"
          accessibilityState={{ expanded: !collapsed }}
          accessibilityLabel={`${collapsed ? "Expand" : "Collapse"} ${title.toLowerCase()} group, ${rows.length} files`}
          style={styles.groupToggle}
        >
          {collapsed ? (
            <ChevronRight size={12} strokeWidth={1.75} color={tokens.ink3} />
          ) : (
            <ChevronDown size={12} strokeWidth={1.75} color={tokens.ink3} />
          )}
          <Text style={[typeScale.section, styles.groupLabel, { color: tokens.ink3 }]} numberOfLines={1}>
            {title} · {rows.length}
          </Text>
        </Pressable>
        <View style={styles.groupActions}>
          {onDiscard ? (
            <GroupHeaderButton
              label="discard"
              accessibilityLabel={`discard all ${rows.length} ${title.toLowerCase()} files`}
              busy={busy}
              tone="danger"
              onPress={() => onDiscard(paths, discardKind)}
            />
          ) : null}
          {onGroupAction ? (
            <GroupHeaderButton
              label={`${actionLabel} all`}
              accessibilityLabel={`${actionLabel} all ${rows.length} ${title.toLowerCase()} files`}
              busy={busy}
              onPress={() => onGroupAction(paths)}
            />
          ) : null}
        </View>
      </View>
      {collapsed
        ? null
        : rows.map((row) => (
            <FileRow
              key={`${staged ? "s" : "w"}:${row.path}`}
              row={row}
              staged={staged}
              selected={selected?.path === row.path && selected.staged === staged}
              charBudget={charBudget}
              busy={busy}
              onSelect={() => onSelect({ path: row.path, staged })}
              onToggleIndex={onRowAction ? () => onRowAction(row.path) : null}
              onDiscard={onDiscard ? () => onDiscard([row.path], discardKind) : null}
            />
          ))}
    </View>
  );
}

function ConflictGroup({
  rows,
  charBudget,
  collapsed,
  onToggleCollapsed,
  onResolve,
}: {
  rows: GitFileRow[];
  charBudget: number;
  collapsed: boolean;
  onToggleCollapsed: (id: string) => void;
  onResolve: ((paths: string[]) => void) | null;
}) {
  const tokens = useTokens();
  if (rows.length === 0) return null;
  const paths = rows.map((row) => row.path);
  return (
    <View
      style={[
        styles.group,
        styles.conflicts,
        { borderColor: hexToRgba(tokens.danger, 0.4), backgroundColor: hexToRgba(tokens.danger, 0.06), borderRadius: radii.radius4 },
      ]}
    >
      <View style={styles.groupHeader}>
        <Pressable
          onPress={() => onToggleCollapsed("conflicts")}
          accessibilityRole="button"
          accessibilityState={{ expanded: !collapsed }}
          accessibilityLabel={`${collapsed ? "Expand" : "Collapse"} conflicts group, ${rows.length} files`}
          style={styles.groupToggle}
        >
          {collapsed ? (
            <ChevronRight size={12} strokeWidth={1.75} color={tokens.danger} />
          ) : (
            <ChevronDown size={12} strokeWidth={1.75} color={tokens.danger} />
          )}
          <Text style={[typeScale.section, styles.groupLabel, { color: tokens.danger }]} numberOfLines={1}>
            CONFLICTS · {rows.length}
          </Text>
        </Pressable>
      </View>
      {collapsed
        ? null
        : rows.map((row) => (
            <FileRow
              key={`c:${row.path}`}
              row={row}
              staged={false}
              selected={false}
              charBudget={charBudget}
              busy={false}
              onSelect={null}
              onToggleIndex={null}
              onDiscard={null}
            />
          ))}
      {!collapsed && onResolve ? (
        <Pressable
          onPress={() => onResolve(paths)}
          accessibilityRole="button"
          accessibilityLabel={`Resolve ${rows.length} conflicted file${rows.length === 1 ? "" : "s"} with Forge`}
          style={[styles.resolve, { borderColor: tokens.accent, borderRadius: radii.radius4 }]}
        >
          <Sparkles size={12} strokeWidth={1.75} color={tokens.accent} />
          <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>Resolve with Forge</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export function GitFileList({
  status,
  selected,
  onSelect,
  onStage,
  onUnstage,
  onDiscard,
  onResolveConflicts,
  busy,
}: GitFileListProps) {
  const tokens = useTokens();
  const [width, setWidth] = useState(0);
  const [collapsedGroups, setCollapsedGroups] = useState<ReadonlySet<string>>(new Set());
  const charBudget = Math.max(
    10,
    Math.floor((Math.max(width, 180) - ROW_CHROME_PX) / (PATH_FONT_SIZE * MONO_ADVANCE)),
  );
  const toggleCollapsed = (id: string) =>
    setCollapsedGroups((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  return (
    <ScrollView
      style={styles.list}
      contentContainerStyle={styles.listContent}
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
    >
      <ConflictGroup
        rows={status.conflicted ?? []}
        charBudget={charBudget}
        collapsed={collapsedGroups.has("conflicts")}
        onToggleCollapsed={toggleCollapsed}
        onResolve={onResolveConflicts}
      />
      <Group
        id="staged"
        title="STAGED"
        rows={status.staged}
        staged
        actionLabel="unstage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        collapsed={collapsedGroups.has("staged")}
        onToggleCollapsed={toggleCollapsed}
        onSelect={onSelect}
        onGroupAction={onUnstage}
        onRowAction={onUnstage ? (path) => onUnstage([path]) : null}
        discardKind="tracked"
        onDiscard={null}
      />
      <Group
        id="unstaged"
        title="UNSTAGED"
        rows={status.unstaged}
        staged={false}
        actionLabel="stage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        collapsed={collapsedGroups.has("unstaged")}
        onToggleCollapsed={toggleCollapsed}
        onSelect={onSelect}
        onGroupAction={onStage}
        onRowAction={onStage ? (path) => onStage([path]) : null}
        discardKind="tracked"
        onDiscard={onDiscard}
      />
      <Group
        id="untracked"
        title="UNTRACKED"
        rows={status.untracked}
        staged={false}
        actionLabel="stage"
        selected={selected}
        charBudget={charBudget}
        busy={busy}
        collapsed={collapsedGroups.has("untracked")}
        onToggleCollapsed={toggleCollapsed}
        onSelect={onSelect}
        onGroupAction={onStage}
        onRowAction={onStage ? (path) => onStage([path]) : null}
        discardKind="untracked"
        onDiscard={onDiscard}
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
  conflicts: { borderWidth: 1, paddingTop: space.space8, paddingHorizontal: 4, paddingBottom: 6 },
  groupHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space8,
    paddingHorizontal: 2,
    paddingBottom: 6,
  },
  groupToggle: { flexDirection: "row", alignItems: "center", gap: 3, flexShrink: 1 },
  groupActions: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  groupLabel: { fontFamily: monoFamily.regular, flexShrink: 1 },
  rowWrap: { flexDirection: "row", alignItems: "center", gap: 2 },
  row: {
    flex: 1,
    height: ROW_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: space.space8,
    borderWidth: 1,
  },
  statusLetter: { fontSize: 10, lineHeight: 14, fontFamily: monoFamily.bold, width: 9 },
  path: { flex: 1, fontSize: PATH_FONT_SIZE, lineHeight: 15, fontFamily: monoFamily.regular },
  count: { fontSize: 9.5, lineHeight: 13, fontFamily: monoFamily.regular },
  rowAction: { width: 20, height: ROW_HEIGHT, alignItems: "center", justifyContent: "center" },
  resolve: {
    marginTop: 6,
    marginHorizontal: 4,
    height: 26,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
    borderWidth: 1,
  },
  truncated: { paddingHorizontal: 6, paddingTop: space.space8 },
});
