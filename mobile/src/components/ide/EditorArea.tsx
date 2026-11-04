// The centre: editor tabs (a single click previews, a double click or an edit pins), the file
// header with its review actions, and the diff or file below it.
import { Check, ClipboardCheck, History, MessageSquare, Pencil, Undo2, X } from "lucide-react-native";
import React, { Suspense, useCallback, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated, { FadeIn } from "react-native-reanimated";

import { useDiscardPaths } from "../../lib/gitActions";
import { useGitHunk } from "../../lib/queries";
import { durations } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { useToast } from "../ds/ToastHost";
import { FileTypeIcon } from "../git/FileTypeIcon";
import { discardMessage } from "../git/gitReviewSupport";
import { classifyHunkError } from "../git/hunkActions";
import type { EditorTab } from "./editorTabsModel";
import { useIde } from "./IdeContext";
import { requestEditorFocus, setWhyTrace, useDirtyFiles, useWhyTraceOn, dirtyKey } from "./ideSignals";
import { IdeIconButton } from "./ideUi";
import { ReviewDraftBar } from "./ReviewDraft";
import { undoRequests, undoStepsForFile } from "./hunkUndo";
import { useFileDiff } from "./useFileDiff";
import { rowBackground } from "./rowStyle";
import type { DiffMode } from "./diffRows";

const DiffView = React.lazy(() => import("./DiffView").then((m) => ({ default: m.DiffView })));
const FileView = React.lazy(() => import("./FileView").then((m) => ({ default: m.FileView })));

const TAB_HEIGHT = 34;

const Tab = React.memo(function Tab({
  tab,
  active,
  onActivate,
  onPin,
  onClose,
}: {
  tab: EditorTab;
  active: boolean;
  onActivate: (id: string) => void;
  onPin: (id: string) => void;
  onClose: (id: string) => void;
}) {
  const tokens = useTokens();
  const dirty = useDirtyFiles().has(dirtyKey(tab.sessionId, tab.path));
  const name = tab.path.slice(tab.path.lastIndexOf("/") + 1);
  const press = useCallback(() => onActivate(tab.id), [onActivate, tab.id]);
  const pin = useMemo(() => ({ onDoubleClick: () => onPin(tab.id) }) as object, [onPin, tab.id]);
  return (
    <Pressable
      onPress={press}
      {...pin}
      accessibilityRole="tab"
      accessibilityState={{ selected: active }}
      accessibilityLabel={`${name}${tab.kind === "diff" ? " changes" : ""}${tab.preview ? ", preview" : ""}`}
      style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [
        styles.tab,
        {
          backgroundColor: active ? tokens.bg0 : rowBackground(tokens, { hovered }),
          borderRightColor: tokens.border,
        },
      ]}
    >
      {active ? <View style={[styles.tabTop, { backgroundColor: tokens.ink }]} /> : null}
      <FileTypeIcon path={tab.path} />
      <Text numberOfLines={1} style={[styles.tabName, { color: active ? tokens.ink : tokens.ink2 }, tab.preview && styles.preview]}>
        {name}
      </Text>
      {tab.kind === "diff" ? <Text style={[styles.tabKind, { color: tokens.ink4 }]}>diff</Text> : null}
      {dirty ? <View accessibilityLabel="Unsaved changes" style={[styles.dirty, { backgroundColor: tokens.ink }]} /> : null}
      <Pressable
        onPress={() => onClose(tab.id)}
        hitSlop={4}
        accessibilityRole="button"
        accessibilityLabel={`Close ${name}`}
        style={styles.tabClose}
      >
        <X size={11} strokeWidth={2} color={tokens.ink3} />
      </Pressable>
    </Pressable>
  );
});

function TabStrip() {
  const tokens = useTokens();
  const { tabs, active, dispatchTabs } = useIde();
  const onActivate = useCallback((id: string) => dispatchTabs({ type: "activate", id }), [dispatchTabs]);
  const onPin = useCallback((id: string) => dispatchTabs({ type: "pin", id }), [dispatchTabs]);
  const onClose = useCallback((id: string) => dispatchTabs({ type: "close", id }), [dispatchTabs]);
  if (tabs.length === 0) return <View style={[styles.strip, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }]} />;
  return (
    <View style={[styles.strip, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }]} accessibilityRole="tablist">
      {tabs.map((tab) => (
        <Tab key={tab.id} tab={tab} active={tab.id === active?.id} onActivate={onActivate} onPin={onPin} onClose={onClose} />
      ))}
    </View>
  );
}

function Crumbs({ path }: { path: string }) {
  const tokens = useTokens();
  const parts = path.split("/");
  const base = parts.pop() ?? path;
  return (
    <Text numberOfLines={1} style={[styles.crumbs, { color: tokens.ink3 }]}>
      {parts.length > 0 ? `${parts.join(" › ")} › ` : ""}
      <Text style={{ color: tokens.ink, fontFamily: monoFamily.medium }}>{base}</Text>
    </Text>
  );
}

function FileHeader({ tab, mode, onMode }: { tab: EditorTab; mode: DiffMode | "auto"; onMode: (mode: DiffMode | "auto") => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const { activeRow, toggleReviewed, setScope, focusComposer, sessions, openChange, openFile, changes } = useIde();
  const discard = useDiscardPaths();
  const hunkMutation = useGitHunk();
  const [confirming, setConfirming] = useState(false);
  const agent = sessions.find((row) => row.id === tab.sessionId);
  const reviewed = activeRow?.tier === "reviewed";
  const changedFile = changes.get(tab.sessionId)?.files.find((file) => file.path === tab.path);
  const untracked = changedFile?.bucket === "untracked";
  const worktree = !!agent?.worktree;
  const traceOn = useWhyTraceOn();
  const { file: diffFile, bucket } = useFileDiff(tab.sessionId, tab.path, worktree);
  // A revert can be put back only when the diff is the working tree's own and rebuilds from its hunks.
  const undoSteps = !worktree && bucket === "unstaged" && diffFile ? undoStepsForFile(diffFile) : [];
  const undoable = undoSteps.length > 0;

  const undo = async () => {
    try {
      for (const request of undoRequests(tab.sessionId, tab.path, undoSteps)) await hunkMutation.mutateAsync(request);
      toast.show("Restored", { tone: "success" });
    } catch (error) {
      toast.show(`Could not undo: ${classifyHunkError(error).message}`, { tone: "danger" });
    }
  };
  const revert = async () => {
    setConfirming(false);
    try {
      const result = await discard.mutateAsync({ session: tab.sessionId, paths: [tab.path] });
      const name = tab.path.slice(tab.path.lastIndexOf("/") + 1);
      if (result.discarded > 0 && undoable) toast.show(`Reverted ${name}`, { duration: 9000, action: { label: "Undo", onPress: () => void undo() } });
      else toast.show(`Reverted ${name}`, { tone: result.discarded > 0 ? "neutral" : "danger" });
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not revert this file.", { tone: "danger" });
    }
  };
  const copy = discardMessage([tab.path], untracked ? "untracked" : "tracked");

  return (
    <View style={[styles.header, { borderBottomColor: tokens.hairline }]}>
      <View style={styles.crumbWrap}>
        <Crumbs path={tab.path} />
        {agent ? (
          <View style={[styles.chip, { borderColor: tokens.border }]}>
            <Text numberOfLines={1} style={[styles.chipText, { color: tokens.ink2 }]}>
              {agent.title?.trim() || `session ${agent.id.slice(0, 8)}`}
            </Text>
          </View>
        ) : null}
      </View>
      <View style={styles.actions}>
        <IdeIconButton
          icon={<History size={14} strokeWidth={1.75} color={traceOn ? tokens.ink : tokens.ink2} />}
          label={traceOn ? "Hide why-trace (w)" : "Why-trace: which turn wrote each line (w)"}
          active={traceOn}
          onPress={() => setWhyTrace(!traceOn)}
        />
        {tab.kind === "diff" ? (
          <>
            <View style={[styles.modes, { borderColor: tokens.border }]}>
              {(["auto", "unified", "split"] as const).map((value) => (
                <Pressable
                  key={value}
                  onPress={() => onMode(value)}
                  accessibilityRole="button"
                  accessibilityState={{ selected: mode === value }}
                  style={[styles.mode, mode === value && { backgroundColor: rowBackground(tokens, { selected: true }) }]}
                >
                  <Text style={[styles.modeText, { color: mode === value ? tokens.ink : tokens.ink3 }]}>{value === "auto" ? "Auto" : value === "unified" ? "Unified" : "Split"}</Text>
                </Pressable>
              ))}
            </View>
            <IdeIconButton icon={<Undo2 size={14} strokeWidth={1.75} color={tokens.ink2} />} label="Revert this file" onPress={() => (undoable ? void revert() : setConfirming(true))} />
            <IdeIconButton
              icon={<Pencil size={14} strokeWidth={1.75} color={tokens.ink2} />}
              label="Edit this file (e)"
              onPress={() => {
                openFile(tab.sessionId, tab.path, true);
                requestEditorFocus();
              }}
            />
            <IdeIconButton
              icon={<MessageSquare size={14} strokeWidth={1.75} color={tokens.ink2} />}
              label="Open in agent"
              onPress={() => {
                setScope(tab.sessionId);
                focusComposer();
              }}
            />
            {activeRow ? (
              <Pressable
                onPress={() => toggleReviewed(activeRow.sessionId, activeRow.path, activeRow.file.hash)}
                accessibilityRole="button"
                accessibilityLabel={reviewed ? "Reviewed. Mark not reviewed" : "Mark reviewed"}
                style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [
                  styles.reviewButton,
                  { borderColor: tokens.borderStrong, backgroundColor: rowBackground(tokens, { hovered, selected: reviewed }) },
                ]}
              >
                {reviewed ? <Check size={12} strokeWidth={2.25} color={tokens.ink2} /> : <ClipboardCheck size={12} strokeWidth={1.75} color={tokens.ink} />}
                <Text style={[styles.modeText, { color: reviewed ? tokens.ink2 : tokens.ink }]}>{reviewed ? "Reviewed" : "Mark reviewed"}</Text>
              </Pressable>
            ) : null}
          </>
        ) : changedFile ? (
          <Pressable onPress={() => openChange(tab.sessionId, tab.path)} accessibilityRole="button" style={[styles.reviewButton, { borderColor: tokens.borderStrong }]}>
            <Text style={[styles.modeText, { color: tokens.ink }]}>View changes</Text>
          </Pressable>
        ) : null}
      </View>
      <ConfirmDialog
        visible={confirming}
        title={copy.title}
        message={copy.message}
        confirmLabel={untracked ? "Delete" : "Revert"}
        destructive
        loading={discard.isPending}
        onConfirm={() => void revert()}
        onCancel={() => setConfirming(false)}
      />
    </View>
  );
}

function EmptyEditor() {
  const tokens = useTokens();
  const { rows, openChange, sessions, changesLoading } = useIde();
  const todo = rows.filter((row) => row.tier !== "reviewed");
  const agents = new Set(todo.map((row) => row.sessionId)).size;
  const first = todo[0];
  let message = "Pick a file from Changes to review it here.";
  if (sessions.length === 0) message = "Start an agent on this project, then review what it changes here.";
  else if (!changesLoading && rows.length === 0) message = "Nothing to review. When an agent changes a file, it shows up in Changes.";
  else if (!changesLoading && todo.length === 0) message = "All caught up. Every changed file is reviewed.";
  else if (first) message = `${todo.length} ${todo.length === 1 ? "file" : "files"} from ${agents} ${agents === 1 ? "agent" : "agents"} to review.`;
  return (
    <View style={styles.empty}>
      <EmptyState
        icon={ClipboardCheck}
        message={message}
        action={
          first ? (
            <View style={styles.emptyAction}>
              <Button label="Review first file" variant="secondary" onPress={() => openChange(first.sessionId, first.path)} />
              <Text style={[styles.emptyHint, { color: tokens.ink4 }]}>or press j</Text>
            </View>
          ) : undefined
        }
      />
    </View>
  );
}

export function EditorArea() {
  const tokens = useTokens();
  const { active, sessions } = useIde();
  const [mode, setMode] = useState<DiffMode | "auto">("auto");
  const worktree = !!(active && sessions.find((row) => row.id === active.sessionId)?.worktree);
  return (
    <View style={[styles.root, { backgroundColor: tokens.bg0 }]}>
      <TabStrip />
      {active ? (
        <>
          <FileHeader tab={active} mode={mode} onMode={setMode} />
          <Animated.View key={active.id} entering={FadeIn.duration(durations.fast)} style={styles.body}>
            <Suspense fallback={<View style={styles.body} />}>
              {active.kind === "diff" ? (
                <DiffView sessionId={active.sessionId} path={active.path} worktree={worktree} mode={mode === "auto" ? undefined : mode} />
              ) : (
                <FileView sessionId={active.sessionId} path={active.path} />
              )}
            </Suspense>
          </Animated.View>
          <ReviewDraftBar sessionId={active.sessionId} />
        </>
      ) : (
        <EmptyEditor />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minWidth: 0, minHeight: 0 },
  body: { flex: 1, minHeight: 0 },
  strip: { height: TAB_HEIGHT, flexDirection: "row", alignItems: "stretch", borderBottomWidth: StyleSheet.hairlineWidth },
  tab: { maxWidth: 220, minWidth: 96, flexDirection: "row", alignItems: "center", gap: 6, paddingLeft: space.space12, paddingRight: space.space8, borderRightWidth: StyleSheet.hairlineWidth },
  tabTop: { position: "absolute", top: 0, left: 0, right: 0, height: 1.5 },
  tabName: { flexShrink: 1, fontFamily: sansFamilyFor(500), fontSize: 12 },
  preview: { fontStyle: "italic" },
  dirty: { width: 6, height: 6, borderRadius: 3 },
  tabKind: { fontFamily: monoFamily.regular, fontSize: 9.5 },
  tabClose: { width: 16, height: 16, borderRadius: radii.radius4, alignItems: "center", justifyContent: "center" },
  header: { minHeight: 40, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12, paddingHorizontal: space.space16, borderBottomWidth: StyleSheet.hairlineWidth },
  crumbWrap: { flex: 1, minWidth: 0, overflow: "hidden", flexDirection: "row", alignItems: "center", gap: space.space8 },
  crumbs: { flexShrink: 1, fontFamily: monoFamily.regular, fontSize: 11.5 },
  chip: { flexShrink: 1, minWidth: 0, maxWidth: 160, borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: 6, paddingVertical: 1 },
  chipText: { fontFamily: sansFamilyFor(500), fontSize: 10.5 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  modes: { flexDirection: "row", borderWidth: 1, borderRadius: radii.radius4, overflow: "hidden", marginRight: space.space4 },
  mode: { paddingHorizontal: 7, height: 22, justifyContent: "center" },
  modeText: { fontFamily: sansFamilyFor(500), fontSize: 10.5 },
  reviewButton: { height: 24, flexDirection: "row", alignItems: "center", gap: 5, paddingHorizontal: space.space8, marginLeft: space.space4, borderWidth: 1, borderRadius: radii.radius4 },
  empty: { flex: 1, alignItems: "center", justifyContent: "center", gap: space.space8 },
  emptyAction: { alignItems: "center", gap: space.space8 },
  emptyHint: { fontFamily: monoFamily.regular, fontSize: 10.5 },
});
