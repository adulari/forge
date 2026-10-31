// One dense (28px) session row of the desktop sidebar: status dot, provider mark, title, the
// failed-turn dot, branch / change / pane badges and the one right-hand metric. Forks hang under
// the session they came from (indented, with a hairline), double-click or the context menu
// renames in place, and the context menu also forks, opens a worktree thread, archives and
// deletes, with the reason on every action it has to refuse.
import { router } from "expo-router";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Platform, Pressable, StyleSheet, Text, TextInput, View, type GestureResponderEvent } from "react-native";

import { ApiError, type SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { providerFamily, providerFamilyLabel, providerIdOf } from "../../lib/providerFamily";
import { getResolvedShortcut, shortcutBindingFromEvent, shortcutBindingsEqual } from "../../lib/shortcuts/bindings";
import { useArchiveSession, useCreateSession, useDeleteSession, useMarkSessionDone, usePrefetchSessionHistory, useRenameSession, useStarSession } from "../../lib/queries";
import { diffLabel, failedTurn, projectRoot, sessionStateWord } from "../../lib/sidebarModel";
import { requestSessionPrewarm } from "../../lib/sessionPrewarm";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type StatusDotState } from "../../theme/tokens";
import { formatCost, formatRelativeTime, tabularNums, type as typeScale } from "../../theme/typography";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { ProviderGlyph } from "../ds/ProviderGlyph";
import { useRelativeClock } from "../ds/relativeClock";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { ForkSheet } from "../session/ForkSheet";
import { MAX_PANES } from "../workbench/layoutModel";
import type { OpenMode } from "../workbench/WorkbenchProvider";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "./PopoverMenu";
import { useDragSource } from "./sessionDrag";

export const ROW_HEIGHT = 28;
const INDENT = 14;

export interface SidebarSessionRowProps {
  row: SessionRow;
  serverId: string;
  depth: number;
  /** The row owns its checkout (a worktree, or the only session in that directory), so its change
   * count is its own; sessions sharing a directory show the total on the project instead. */
  showDiff: boolean;
  selected: boolean;
  /** 1-based pane number when this session is on screen in a multi-pane layout. */
  paneNumber: number | null;
  paneFocused: boolean;
  /** How many panes are open: decides whether "open in new pane" can add one. */
  paneTotal: number;
  onOpen: (id: string, mode: OpenMode) => void;
  onClosePane: (id: string) => void;
}

const errorText = (error: unknown, fallback: string) => (error instanceof ApiError ? error.message : fallback);

export function SidebarSessionRow({ row, serverId, depth, showDiff, selected, paneNumber, paneFocused, paneTotal, onOpen, onClosePane }: SidebarSessionRowProps) {
  const tokens = useTokens();
  const toast = useToast();
  const { setActive } = useAuth();
  const prefetchHistory = usePrefetchSessionHistory();
  const rename = useRenameSession();
  const archive = useArchiveSession();
  const remove = useDeleteSession();
  const star = useStarSession();
  const markDone = useMarkSessionDone();
  const createSession = useCreateSession();
  const warmSession = useCallback(
    (id: string) => void prefetchHistory(id).then(() => requestSessionPrewarm(id)),
    [prefetchHistory],
  );
  // The list only re-renders when the fleet changes, so a computed "4m" would otherwise freeze.
  useRelativeClock();
  const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
  const title = row.title || `session ${row.id.slice(0, 8)}`;
  const ref = useRef<View>(null);
  const [menu, setMenu] = useState<MenuAnchor | null>(null);
  const [renaming, setRenaming] = useState(false);
  // Enter and the blur it causes both finish a rename; only the first counts.
  const renameSettled = useRef(true);
  const [draft, setDraft] = useState(title);
  const [forkOpen, setForkOpen] = useState(false);
  const [confirm, setConfirm] = useState<"archive" | "delete" | null>(null);
  useDragSource(ref, { kind: "session", id: row.id });

  const terminal = row.terminal ?? row.read_only ?? false;
  const providerId = providerIdOf(row);
  const family = providerFamilyLabel(providerFamily(providerId));
  const failed = failedTurn(row);
  const diff = showDiff ? diffLabel(row.diff_added, row.diff_removed) : null;

  const open = (mode: OpenMode) => {
    if (serverId) setActive(serverId);
    onOpen(row.id, mode);
  };
  const onPress = (event: GestureResponderEvent) => {
    const modifiers = event.nativeEvent as unknown as { altKey?: boolean; shiftKey?: boolean };
    open(modifiers.altKey || modifiers.shiftKey ? "new" : "focused");
  };

  const rowTitle = row.title;
  const startRename = useCallback(() => {
    setDraft(rowTitle || "");
    // The context menu's modal gives focus back as it closes; entering edit mode in the same tick
    // would see that as the input losing focus and end the rename before it began.
    setTimeout(() => {
      renameSettled.current = false;
      setRenaming(true);
    }, 80);
  }, [rowTitle]);
  const finishRename = (commit: boolean) => {
    if (renameSettled.current) return;
    renameSettled.current = true;
    setRenaming(false);
    const next = draft.trim();
    if (!commit || !next || next === row.title) return;
    rename.mutate(
      { id: row.id, title: next },
      { onError: (error) => toast.show(errorText(error, "could not rename the session"), { tone: "danger" }) },
    );
  };

  const openInWorktree = () => {
    createSession.mutate(
      { cwd: projectRoot(row.cwd), worktree: true, title: `${title} (worktree)` },
      {
        onSuccess: (created) => {
          if (serverId) setActive(serverId);
          onOpen(created.id, "focused");
        },
        onError: (error) => toast.show(errorText(error, "could not start a worktree thread"), { tone: "danger" }),
      },
    );
  };

  const runConfirmed = () => {
    const action = confirm;
    setConfirm(null);
    if (action === "archive") {
      archive.mutate(row.id, { onError: (error) => toast.show(errorText(error, "archive failed"), { tone: "danger" }) });
    } else if (action === "delete") {
      remove.mutate(row.id, {
        onSuccess: () => {
          if (selected) router.replace("/");
        },
        onError: (error) => toast.show(errorText(error, "could not delete the session"), { tone: "danger" }),
      });
    }
  };

  const lifecycleReason = terminal ? "open in a terminal: run /delete there, or close the chat first" : undefined;
  const menuEntries: MenuEntry[] = [
    { kind: "item", id: "open", label: "Open", onPress: () => open("focused") },
    {
      kind: "item",
      id: "split",
      label: "Open in split",
      hint: paneTotal >= MAX_PANES ? "replaces last" : undefined,
      onPress: () => open("new"),
    },
    ...(paneNumber != null && paneTotal > 1
      ? ([{ kind: "item", id: "close", label: `Close pane ${paneNumber}`, onPress: () => onClosePane(row.id) }] as MenuEntry[])
      : []),
    { kind: "separator", id: "sep-edit" },
    { kind: "item", id: "rename", label: "Rename", hint: "double-click", onPress: startRename },
    { kind: "item", id: "fork", label: "Fork…", disabled: terminal, hint: lifecycleReason, onPress: () => setForkOpen(true) },
    {
      kind: "item",
      id: "worktree",
      label: "Open in worktree",
      disabled: !!row.worktree,
      hint: row.worktree ? "already in one" : undefined,
      onPress: openInWorktree,
    },
    {
      kind: "item",
      id: "star",
      label: row.starred ? "Unstar" : "Star",
      onPress: () =>
        star.mutate(
          { id: row.id, starred: !row.starred },
          { onError: (error) => toast.show(errorText(error, "could not update the session"), { tone: "danger" }) },
        ),
    },
    {
      kind: "item",
      id: "done",
      label: row.done_at != null ? "Reopen" : "Mark done",
      onPress: () =>
        markDone.mutate(
          { id: row.id, done: row.done_at == null },
          { onError: (error) => toast.show(errorText(error, "could not update the session"), { tone: "danger" }) },
        ),
    },
    { kind: "separator", id: "sep-end" },
    { kind: "item", id: "archive", label: "Archive", disabled: terminal, hint: lifecycleReason, onPress: () => setConfirm("archive") },
    { kind: "item", id: "delete", label: "Delete…", disabled: terminal, hint: lifecycleReason, onPress: () => setConfirm("delete") },
  ];

  const webHandlers =
    Platform.OS === "web"
      ? ({
          onContextMenu: (e: { preventDefault: () => void; clientX: number; clientY: number }) => {
            e.preventDefault();
            setMenu({ top: e.clientY, left: e.clientX });
          },
        } as Record<string, unknown>)
      : null;

  // react-native-web forwards neither double-click nor raw key handlers to the row, so rename's
  // two shortcuts attach to the DOM node directly.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = ref.current as unknown as HTMLElement | null;
    if (!node?.addEventListener) return;
    const onDoubleClick = (event: MouseEvent) => {
      event.preventDefault();
      startRename();
    };
    node.setAttribute("data-focus-zone", "sidebar");
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.target !== node) return;
      if (event.key === "F2") {
        event.preventDefault();
        startRename();
        return;
      }
      const chord = shortcutBindingFromEvent(event);
      if (!chord) return;
      if (shortcutBindingsEqual(chord, getResolvedShortcut("sidebar.rename"))) {
        event.preventDefault();
        startRename();
      } else if (!terminal && shortcutBindingsEqual(chord, getResolvedShortcut("sidebar.archive"))) {
        event.preventDefault();
        setConfirm("archive");
      }
    };
    node.addEventListener("dblclick", onDoubleClick);
    node.addEventListener("keydown", onKeyDown);
    return () => {
      node.removeEventListener("dblclick", onDoubleClick);
      node.removeEventListener("keydown", onKeyDown);
    };
  }, [startRename, terminal]);

  const metric =
    state === "busy" ? formatCost(row.cost_usd) : formatRelativeTime(row.last_activity * 1000);
  const metricColor = state === "busy" ? tokens.ink2 : state === "waiting" ? tokens.ink3 : tokens.ink4;
  const indent = depth * INDENT;

  return (
    <>
      <Pressable
        ref={ref}
        onPress={renaming ? undefined : onPress}
        onHoverIn={() => warmSession(row.id)}
        onPressIn={() => void prefetchHistory(row.id)}
        accessibilityRole="button"
        accessibilityLabel={`${title}, ${sessionStateWord(row)}, ${family}${diff ? `, ${diff} lines` : ""}${paneNumber != null && paneTotal > 1 ? `, open in pane ${paneNumber}` : ""}${depth > 0 ? ", fork" : ""}${row.starred ? ", starred" : ""}${row.done_at != null ? ", done" : ""}${row.pr_watch ? ", watching its pull request" : ""}`}
        accessibilityHint="Opens in the focused pane. Right-click or double-click for more, or drag onto a pane."
        style={[
          styles.row,
          { paddingLeft: space.space8 + indent },
          (row.waiting || selected || paneNumber != null) && { backgroundColor: tokens.selection },
          paneNumber != null && paneTotal > 1 && paneFocused && { borderLeftWidth: 2, borderLeftColor: tokens.accent },
        ]}
        {...(webHandlers as object | null)}
      >
        {depth > 0 ? <View pointerEvents="none" style={[styles.branchLine, { left: space.space8 + indent - INDENT + 3, backgroundColor: tokens.border }]} /> : null}
        <StatusDot state={state} size={6} />
        <ProviderGlyph provider={providerId} size={12} color={state === "idle" ? tokens.ink3 : tokens.ink2} accessibilityLabel={null} />
        {renaming ? (
          <TextInput
            autoFocus
            selectTextOnFocus
            value={draft}
            onChangeText={setDraft}
            onSubmitEditing={() => finishRename(true)}
            onBlur={() => finishRename(true)}
            onKeyPress={(event) => {
              if (event.nativeEvent.key === "Escape") finishRename(false);
            }}
            accessibilityLabel={`Rename ${title}`}
            style={[typeScale.body, styles.renameInput, { color: tokens.ink, borderColor: tokens.accent, backgroundColor: tokens.bg0 }]}
          />
        ) : (
          <Text style={[typeScale.body, styles.title, { color: state === "idle" ? tokens.ink2 : tokens.ink }]} numberOfLines={1}>
            {title}
          </Text>
        )}
        {failed ? (
          <View accessibilityRole="image" accessibilityLabel="last turn failed" style={[styles.failedDot, { backgroundColor: tokens.danger }]} />
        ) : null}
        {row.starred ? <Text accessibilityElementsHidden style={[typeScale.monoMeta, { color: tokens.accent }]}>★</Text> : null}
        {row.pr_watch ? <Text accessibilityElementsHidden style={[typeScale.monoMeta, { color: tokens.accent }]}>◉</Text> : null}
        {row.worktree ? <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>⑂</Text> : null}
        {diff ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]} numberOfLines={1}>
            {diff.split(" ")[0]}
          </Text>
        ) : null}
        {paneNumber != null && paneTotal > 1 ? (
          <Text style={[typeScale.monoMeta, tabularNums, styles.paneBadge, { color: tokens.accent, borderColor: tokens.accent }]}>
            {paneNumber}
          </Text>
        ) : null}
        <Text style={[typeScale.monoMeta, tabularNums, { color: metricColor }]}>{metric}</Text>
      </Pressable>
      <PopoverMenu anchor={menu} entries={menuEntries} onClose={() => setMenu(null)} accessibilityLabel={`${title} actions`} width={248} />
      {forkOpen ? <ForkSheet visible onClose={() => setForkOpen(false)} sessionId={row.id} onForked={(id) => onOpen(id, "focused")} /> : null}
      <ConfirmDialog
        visible={confirm != null}
        title={confirm === "delete" ? "Delete this session permanently?" : "Archive session"}
        message={
          confirm === "delete"
            ? `"${title}" and its transcript will be deleted. This cannot be undone.`
            : `Stop and hide "${title}". History is kept.`
        }
        confirmLabel={confirm === "delete" ? "Delete" : "Archive"}
        destructive={confirm === "delete"}
        onConfirm={runConfirmed}
        onCancel={() => setConfirm(null)}
      />
    </>
  );
}

const styles = StyleSheet.create({
  row: {
    height: ROW_HEIGHT,
    borderRadius: radii.radius4,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingRight: space.space8,
  },
  title: { flex: 1, fontSize: 11.5 },
  renameInput: {
    flex: 1,
    fontSize: 11.5,
    height: 22,
    paddingHorizontal: 4,
    borderWidth: 1,
    borderRadius: radii.radius4,
  },
  failedDot: { width: 5, height: 5, borderRadius: 3 },
  branchLine: { position: "absolute", top: 0, bottom: 0, width: StyleSheet.hairlineWidth },
  paneBadge: { borderWidth: 1, borderRadius: 3, paddingHorizontal: 3, minWidth: 14, textAlign: "center" },
});
