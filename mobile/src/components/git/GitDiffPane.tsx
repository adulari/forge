// Machined git review dock - the diff column (docs/design/machined "Forge Machined - Desktop.dc.html"
// L265-308), now one scroll of collapsible file cards instead of one file at a time: a toolbar
// (split/unified, hide whitespace, collapse/expand all) above a card per changed file. Picking a
// file in the list opens its card and scrolls it into view. Card bodies load only while open, so
// a hundred-file change costs a hundred headers, not a hundred diffs.
import { ChevronsDownUp, ChevronsUpDown, FileDiff } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { type DiffFindLine, type DiffViewMode, GitDiffCard } from "./GitDiffCard";
import { FindBar } from "../find/FindBar";
import { findController, findFocusProps } from "../../lib/find/findController";
import { matchOffsets, steppedIndex } from "../../lib/find/matching";
import { LINE_HEIGHT } from "./diffLines";
import {
  CARD_RENDER_LIMIT,
  type DiffEntry,
  EXPAND_ALL_LIMIT,
  defaultExpandedKeys,
} from "./gitDiffEntries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";

export type { DiffViewMode } from "./GitDiffCard";

/** Below this the two columns are narrower than a typical code line, so split is not offered. */
const SPLIT_MIN_WIDTH = 560;
/** Where a card's first line starts below its top edge, and how much context Find leaves above a
 * hit. Approximate: hunk headers add a row each, so a hit deep in a file can land a little low. */
const FIND_CARD_HEADER = 40;
const FIND_LEAD = 72;

export interface GitDiffFocusRequest {
  key: string;
  /** Changes on every request so picking the same file twice scrolls twice. */
  nonce: number;
}

export interface GitDiffPaneProps {
  sessionId: string;
  entries: DiffEntry[];
  /** Conflicted paths have no card; with only those left the empty state says why. */
  conflictCount: number;
  focusRequest: GitDiffFocusRequest | null;
  ignoreWhitespace: boolean;
  onToggleWhitespace: () => void;
}

function ToolbarToggle({
  label,
  active,
  disabled,
  role,
  hint,
  onPress,
}: {
  label: string;
  active: boolean;
  disabled?: boolean;
  role: "tab" | "switch";
  hint?: string;
  onPress: () => void;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      accessibilityRole={role}
      accessibilityState={role === "tab" ? { selected: active, disabled } : { checked: active, disabled }}
      accessibilityLabel={label}
      accessibilityHint={hint}
      style={[
        styles.chip,
        {
          borderRadius: radii.radiusSegmentInner,
          borderColor: active ? tokens.border : "transparent",
          backgroundColor: active ? tokens.bg3 : "transparent",
        },
      ]}
    >
      <Text style={[typeScale.monoMeta, { color: disabled ? tokens.ink4 : active ? tokens.ink2 : tokens.ink3 }]}>
        {label}
      </Text>
    </Pressable>
  );
}

export function GitDiffPane({
  sessionId,
  entries,
  conflictCount,
  focusRequest,
  ignoreWhitespace,
  onToggleWhitespace,
}: GitDiffPaneProps) {
  const tokens = useTokens();
  const [mode, setMode] = useState<DiffViewMode>("split");
  const [width, setWidth] = useState(0);
  const [overrides, setOverrides] = useState<Record<string, boolean>>({});
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const [renderLimit, setRenderLimit] = useState(CARD_RENDER_LIMIT);
  const scroller = useRef<ScrollView>(null);
  const positions = useRef(new Map<string, number>());

  const lineIndex = useRef(new Map<string, DiffFindLine[]>());
  const onLines = useCallback((key: string, lines: DiffFindLine[]) => {
    lineIndex.current.set(key, lines);
  }, []);
  const findRefs = useRef({ entries, scroll: (_y: number) => {}, show: (_key: string) => {} });

  const defaults = useMemo(() => defaultExpandedKeys(entries), [entries]);
  const isExpanded = useCallback(
    (key: string) => overrides[key] ?? defaults.has(key),
    [defaults, overrides],
  );

  const toggleCard = useCallback(
    (key: string) =>
      setOverrides((current) => ({ ...current, [key]: !(current[key] ?? defaults.has(key)) })),
    [defaults],
  );

  useEffect(() => {
    if (!focusRequest) return;
    const index = entries.findIndex((entry) => entry.key === focusRequest.key);
    if (index < 0) return;
    setFocusedKey(focusRequest.key);
    setOverrides((current) => ({ ...current, [focusRequest.key]: true }));
    if (index >= renderLimit) setRenderLimit(index + 1);
    const y = positions.current.get(focusRequest.key);
    if (y != null) scroller.current?.scrollTo({ y: Math.max(0, y - space.space8), animated: true });
    // Only a new request should move the scroll; entries/limit changes must not re-fire it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusRequest?.nonce]);

  useEffect(() => {
    findRefs.current = {
      entries,
      scroll: (y) => scroller.current?.scrollTo({ y: Math.max(0, y), animated: false }),
      show: (key) => {
        setFocusedKey(key);
        setOverrides((current) => ({ ...current, [key]: true }));
      },
    };
  });

  useEffect(() => {
    let hits: { key: string; path: string; lineNo: number; row: number }[] = [];
    let index = -1;
    const activate = (next: number) => {
      index = next;
      const hit = hits[next];
      if (!hit) return { count: hits.length, index };
      findRefs.current.show(hit.key);
      const top = positions.current.get(hit.key) ?? 0;
      findRefs.current.scroll(top + FIND_CARD_HEADER + hit.row * LINE_HEIGHT - FIND_LEAD);
      return { count: hits.length, index, detail: `${hit.path}:${hit.lineNo}` };
    };
    return findController.register({
      id: "diff",
      label: "diff",
      search(query, caseSensitive) {
        hits = [];
        for (const entry of findRefs.current.entries) {
          for (const line of lineIndex.current.get(entry.key) ?? []) {
            if (matchOffsets(line.text, query, caseSensitive, 1).length > 0) {
              hits.push({ key: entry.key, path: entry.path, lineNo: line.lineNo, row: line.row });
            }
          }
        }
        return activate(hits.length > 0 ? 0 : -1);
      },
      step: (direction) => activate(steppedIndex(index, hits.length, direction)),
      clear() {
        hits = [];
        index = -1;
        setFocusedKey(null);
      },
    });
  }, []);

  const splitAvailable = width === 0 || width >= SPLIT_MIN_WIDTH;
  const effectiveMode: DiffViewMode = splitAvailable ? mode : "unified";

  const totals = useMemo(
    () =>
      entries.reduce(
        (sum, entry) => ({ adds: sum.adds + entry.row.adds, dels: sum.dels + entry.row.dels }),
        { adds: 0, dels: 0 },
      ),
    [entries],
  );
  const openCount = entries.filter((entry) => isExpanded(entry.key)).length;
  const allOpen = entries.length > 0 && openCount === Math.min(entries.length, EXPAND_ALL_LIMIT);
  const toggleAll = () => {
    const next: Record<string, boolean> = {};
    entries.forEach((entry, index) => {
      next[entry.key] = allOpen ? false : index < EXPAND_ALL_LIMIT;
    });
    setOverrides(next);
  };
  const visible = entries.slice(0, renderLimit);

  return (
    <View style={styles.pane} onLayout={(event) => setWidth(event.nativeEvent.layout.width)} {...findFocusProps("diff")}>
      <FindBar targetId="diff" />
      <View style={[styles.toolbar, { borderBottomColor: tokens.border }]}>
        <Text style={[styles.summary, tabularNums, { color: tokens.ink2 }]} numberOfLines={1}>
          {entries.length} file{entries.length === 1 ? "" : "s"}
        </Text>
        <Text style={[styles.summary, tabularNums, { color: tokens.success }]}>+{totals.adds}</Text>
        <Text style={[styles.summary, tabularNums, { color: tokens.danger }]}>−{totals.dels}</Text>
        <View style={styles.spacer} />
        <Pressable
          onPress={toggleAll}
          disabled={entries.length === 0}
          accessibilityRole="button"
          accessibilityLabel={allOpen ? "Collapse all files" : "Expand all files"}
          accessibilityHint={entries.length > EXPAND_ALL_LIMIT ? `Opens the first ${EXPAND_ALL_LIMIT} files` : undefined}
          hitSlop={6}
          style={styles.iconAction}
        >
          {allOpen ? (
            <ChevronsDownUp size={14} strokeWidth={1.75} color={tokens.ink3} />
          ) : (
            <ChevronsUpDown size={14} strokeWidth={1.75} color={tokens.ink3} />
          )}
        </Pressable>
        <ToolbarToggle
          label="hide whitespace"
          role="switch"
          active={ignoreWhitespace}
          onPress={onToggleWhitespace}
        />
        <View style={styles.toggleGroup}>
          <ToolbarToggle label="split" role="tab" active={effectiveMode === "split"} disabled={!splitAvailable} hint={splitAvailable ? undefined : "the pane is too narrow for two columns"} onPress={() => setMode("split")} />
          <ToolbarToggle label="unified" role="tab" active={effectiveMode === "unified"} onPress={() => setMode("unified")} />
        </View>
      </View>
      {entries.length === 0 ? (
        <EmptyState
          icon={FileDiff}
          message={
            conflictCount > 0
              ? "Conflicted files have no diff. Resolve them, then review what is left."
              : "No changes to review."
          }
        />
      ) : (
        <ScrollView ref={scroller} style={styles.scroll} contentContainerStyle={styles.stack}>
          {visible.map((entry) => (
            <View
              key={entry.key}
              onLayout={(event) => positions.current.set(entry.key, event.nativeEvent.layout.y)}
            >
              <GitDiffCard
                sessionId={sessionId}
                entry={entry}
                expanded={isExpanded(entry.key)}
                focused={focusedKey === entry.key}
                onToggle={toggleCard}
                mode={effectiveMode}
                ignoreWhitespace={ignoreWhitespace}
                width={width}
                onLines={onLines}
              />
            </View>
          ))}
          {entries.length > visible.length ? (
            <Pressable
              onPress={() => setRenderLimit((limit) => limit + CARD_RENDER_LIMIT)}
              accessibilityRole="button"
              accessibilityLabel={`Show ${entries.length - visible.length} more files`}
              style={[styles.more, { borderColor: tokens.border, borderRadius: radii.radius4 }]}
            >
              <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>
                show {entries.length - visible.length} more files
              </Text>
            </Pressable>
          ) : null}
        </ScrollView>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  pane: { flex: 1, minWidth: 0 },
  toolbar: {
    minHeight: 34,
    flexShrink: 0,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    columnGap: 9,
    rowGap: 4,
    paddingVertical: 4,
    paddingHorizontal: space.space12,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  summary: { fontSize: 10.5, lineHeight: 14 },
  spacer: { flex: 1 },
  iconAction: { width: 22, height: 22, alignItems: "center", justifyContent: "center" },
  toggleGroup: { flexDirection: "row", gap: space.space4 },
  chip: { paddingHorizontal: 6, paddingVertical: 1, borderWidth: 1 },
  scroll: { flex: 1 },
  stack: { padding: space.space8, gap: space.space8 },
  more: { alignSelf: "center", borderWidth: 1, paddingHorizontal: space.space12, paddingVertical: 6 },
});
