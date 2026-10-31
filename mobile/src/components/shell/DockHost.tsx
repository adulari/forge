// Machined desktop shell — the dock container (D Split + Usage / D Rail + Terminal / D Git
// Review, docs/design/machined INVENTORY.md): a typed registry of docked panels, each declaring
// where it attaches. Right-edge docks share one resizable column beside the content, switched
// with tabs; the terminal is a resizable bottom strip inside the content column. Sizes are
// per dock kind, clamped to the window, persisted by the workbench, and reset by double-click.
//
// Adding a dock is one registry entry plus the hotkey/menu wiring in app/_layout.tsx — nothing
// else in this file changes.
import { PanelRight, X } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { closeTerminal } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useTerminalPreferences } from "../../lib/terminalPreferences";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import { ActionsDock } from "../actions/ActionsDock";
import { PortsDock } from "../ports/PortsDock";
import { hasActionTerminal, useActionTabInfo } from "../ports/useActionTabInfo";
import { GalleryDock } from "../gallery/GalleryDock";
import { GitReviewDock } from "../git/GitReviewDock";
import { BrowserPreviewDock } from "../preview/BrowserPreviewDock";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { StatusDot } from "../ds/StatusDot";
import { WorkspaceDock } from "../workspace/WorkspaceDock";
import {
  clampBottomHeight,
  clampRightWidth,
} from "../workbench/layoutModel";
import {
  WORKBENCH_SURFACE_DEFINITIONS,
  type WorkbenchSurface,
  type WorkbenchSurfaceKind,
} from "../workbench/model";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { useSessionRow } from "./activeSession";
import { ResizeHandle } from "./ResizeHandle";
import { useDragSource, useDropTarget, type DragPayload } from "./sessionDrag";
import { SessionDock } from "./SessionDocks";
import { TerminalDock } from "./TerminalDock";
import { terminalLabel } from "../ports/actionTerminalNames";
import { markTerminalClosed, tabsForSession, tabTerminalId } from "./terminalTabs";
import { useTerminalActivity } from "./useTerminalActivity";
import { UsageDock } from "./UsageDock";

export type DockKind = WorkbenchSurfaceKind;

/** What a terminal tab says about its shell when the person is not looking at it. */
export interface TabSignal {
  kind: "unseen" | "streaming" | "exited" | "running";
  label: string;
}

function DockTab({
  tab,
  index,
  active,
  signal,
  onActivate,
  onClose,
  onReorder,
}: {
  tab: WorkbenchSurface;
  index: number;
  active: boolean;
  signal?: TabSignal | null;
  onActivate?: (id: string) => void;
  onClose?: (id: string) => void;
  onReorder?: (from: number, to: number) => void;
}) {
  const tokens = useTokens();
  const ref = useRef<View>(null);
  const [over, setOver] = useState(false);
  useDragSource(ref, { kind: "tab", index });
  const accepts = useCallback((payload: DragPayload) => payload.kind === "tab" && payload.index !== index, [index]);
  const drop = useCallback(
    (payload: DragPayload) => {
      if (payload.kind === "tab") onReorder?.(payload.index, index);
    },
    [index, onReorder],
  );
  useDropTarget(ref, accepts, drop, setOver);
  return (
    <View
      ref={ref}
      style={[styles.tab, { borderBottomColor: active ? tokens.accent : "transparent" }, over && { backgroundColor: tokens.bg3 }]}
    >
      <Pressable
        onPress={() => onActivate?.(tab.id)}
        accessibilityRole="tab"
        accessibilityState={{ selected: active }}
        accessibilityLabel={`Open ${tab.title} surface`}
        accessibilityHint="Drag to reorder"
        style={styles.tabLabel}
      >
        <Text style={[typeScale.bodyBold, { color: active ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>
          {tab.title}
        </Text>
        {signal?.kind === "running" ? (
          <View style={styles.runningDot}>
            <StatusDot state="busy" accessibilityLabel={`${tab.title} running`} />
          </View>
        ) : signal?.kind === "streaming" ? (
          <Text
            accessibilityLabel="Printing output"
            style={[typeScale.monoMeta, styles.liveBadge, { color: tokens.accent, backgroundColor: hexToRgba(tokens.accent, 0.12) }]}
          >
            live
          </Text>
        ) : signal?.kind === "unseen" ? (
          <View accessibilityLabel="New output" style={[styles.unseenDot, { backgroundColor: tokens.accent }]} />
        ) : signal?.kind === "exited" ? (
          <Text numberOfLines={1} style={[typeScale.monoMeta, styles.exitLabel, { color: tokens.ink4 }]}>
            {signal.label}
          </Text>
        ) : null}
      </Pressable>
      {onClose ? (
        <Pressable
          onPress={() => onClose(tab.id)}
          accessibilityRole="button"
          accessibilityLabel={`Close ${tab.title} tab`}
          style={styles.tabClose}
        >
          <X size={11} strokeWidth={2} color={tokens.ink4} />
        </Pressable>
      ) : null}
    </View>
  );
}

export interface DockContext {
  sessionId: string | null;
  surface: WorkbenchSurface | null;
  openTerminal: (terminalId: string) => void;
  /** Close the terminal tab whose shell exited on its own. */
  closeFinishedTerminal: () => void;
}

interface DockDefinition {
  render: (ctx: DockContext) => React.ReactNode;
}

function NeedsSession({ what }: { what: string }) {
  return <EmptyState icon={PanelRight} message={`Open a session to see ${what} here.`} />;
}

const DOCK_REGISTRY: Record<DockKind, DockDefinition> = {
  usage: {
    render: () => <UsageDock />,
  },
  git: {
    // The dock follows the focused pane's session unless a future resource tab pins one.
    render: ({ sessionId }) => (sessionId ? <GitReviewDock sessionId={sessionId} /> : <NeedsSession what="its changes" />),
  },
  files: {
    render: ({ sessionId, surface }) => (
      <WorkspaceDock sessionId={sessionId} resourceId={surface?.resourceId ?? null} line={surface?.line ?? null} />
    ),
  },
  preview: {
    render: ({ sessionId, surface }) =>
      sessionId && surface ? <BrowserPreviewDock sessionId={sessionId} surface={surface} /> : <NeedsSession what="its preview" />,
  },
  terminal: {
    render: ({ sessionId, surface, openTerminal, closeFinishedTerminal }) => (
      <TerminalDock
        sessionId={sessionId}
        terminalId={tabTerminalId(surface?.resourceId ?? null)}
        onOpenTerminal={openTerminal}
        surfaceId={surface?.id ?? null}
        onShellExit={closeFinishedTerminal}
      />
    ),
  },
  tasks: { render: ({ sessionId }) => <SessionDock kind="tasks" sessionId={sessionId} /> },
  agents: { render: ({ sessionId }) => <SessionDock kind="agents" sessionId={sessionId} /> },
  plan: { render: ({ sessionId }) => <SessionDock kind="plan" sessionId={sessionId} /> },
  gallery: {
    render: ({ sessionId }) => (sessionId ? <GalleryDock sessionId={sessionId} /> : <NeedsSession what="its images" />),
  },
  actions: { render: ({ sessionId }) => <ActionsDock sessionId={sessionId} /> },
  ports: { render: ({ sessionId }) => <PortsDock sessionId={sessionId} /> },
};

export interface DockHostProps {
  open: boolean;
  dock?: DockKind;
  /** Concrete tab selected by the workbench. `dock` remains as a compact fallback for callers. */
  surface?: WorkbenchSurface | null;
  /** Other open tabs in this placement. They remain mounted in the workbench model when hidden. */
  tabs?: readonly WorkbenchSurface[];
  /** Session the dock acts on — ignored by docks that aren't session-scoped. */
  sessionId?: string | null;
  /** Width already taken by the sidebar, so a right dock can never crowd out the content. */
  occupiedWidth?: number;
  onActivateSurface?: (id: string) => void;
  onCloseSurface?: (id: string) => void;
  /** Drag a tab onto another to move it: `(from, to)` as tab indexes. */
  onReorderSurface?: (from: number, to: number) => void;
  onClose: () => void;
}

export function DockHost({
  open,
  dock = "usage",
  surface = null,
  tabs = [],
  sessionId = null,
  occupiedWidth = 0,
  onActivateSurface,
  onCloseSurface,
  onReorderSurface,
  onClose,
}: DockHostProps) {
  const tokens = useTokens();
  const workbench = useWorkbench();
  const { width: windowWidth, height: windowHeight } = useWindowDimensions();
  const kind = surface?.kind ?? dock;
  const definition = DOCK_REGISTRY[kind];
  const surfaceDefinition = WORKBENCH_SURFACE_DEFINITIONS[kind];
  const effectiveSessionId = surface?.sessionId ?? sessionId;
  const openTerminal = useCallback(
    (terminalId: string) => {
      if (!effectiveSessionId) return;
      workbench.openSurface({
        kind: "terminal",
        sessionId: effectiveSessionId,
        resourceId: terminalId,
        title: terminalLabel(terminalId),
      });
    },
    [effectiveSessionId, workbench],
  );
  const row = useSessionRow(surfaceDefinition.sessionScoped ? effectiveSessionId : null);

  // Terminal tabs belong to the session they were opened for: the focused session shows its own
  // plus the default one, and the daemon's list brings back any it still has running.
  const isTerminal = kind === "terminal";
  const sessionTabs = useMemo(() => (isTerminal ? tabsForSession(tabs, sessionId) : tabs), [isTerminal, sessionId, tabs]);
  const actionTabs = useActionTabInfo(effectiveSessionId, isTerminal && open && hasActionTerminal(sessionTabs));
  const shownTabs = useMemo(
    () =>
      actionTabs.size === 0
        ? sessionTabs
        : sessionTabs.map((tab) => {
            const info = tab.kind === "terminal" ? actionTabs.get(tabTerminalId(tab.resourceId)) : undefined;
            return info ? { ...tab, title: info.name } : tab;
          }),
    [actionTabs, sessionTabs],
  );
  const activeTerminalId = isTerminal ? tabTerminalId(surface?.resourceId ?? null) : null;
  const terminalActivity = useTerminalActivity(
    isTerminal ? effectiveSessionId : null,
    activeTerminalId,
    isTerminal && open && shownTabs.length > 1,
  );
  const { baseUrl } = useAuth();
  const terminalPreferences = useTerminalPreferences();
  const [closing, setClosing] = useState<WorkbenchSurface | null>(null);

  useEffect(() => {
    if (!isTerminal || !surface || !sessionId || surface.sessionId == null || surface.sessionId === sessionId) return;
    const own = shownTabs.find((tab) => tab.kind === "terminal");
    if (own) onActivateSurface?.(own.id);
    else workbench.openSurface({ kind: "terminal" });
  }, [isTerminal, onActivateSurface, sessionId, shownTabs, surface, workbench]);

  const discardTerminal = useCallback(
    (tab: WorkbenchSurface) => {
      const owner = tab.sessionId ?? sessionId;
      if (owner) {
        const terminalId = tabTerminalId(tab.resourceId);
        markTerminalClosed(owner, terminalId);
        if (baseUrl) closeTerminal(baseUrl, owner, terminalId);
      }
      onCloseSurface?.(tab.id);
    },
    [baseUrl, onCloseSurface, sessionId],
  );
  const requestClose = useCallback(
    (id: string) => {
      const tab = shownTabs.find((candidate) => candidate.id === id);
      if (!tab || tab.kind !== "terminal") {
        onCloseSurface?.(id);
        return;
      }
      const stillRunning = terminalActivity.exitCodeOf(tabTerminalId(tab.resourceId)) === undefined;
      if (terminalPreferences.confirmClose && stillRunning) setClosing(tab);
      else discardTerminal(tab);
    },
    [discardTerminal, onCloseSurface, shownTabs, terminalActivity, terminalPreferences.confirmClose],
  );
  const closeFinishedTerminal = useCallback(() => {
    if (!surface) return;
    const owner = surface.sessionId ?? sessionId;
    if (owner) markTerminalClosed(owner, tabTerminalId(surface.resourceId));
    onCloseSurface?.(surface.id);
  }, [onCloseSurface, sessionId, surface]);
  const signalFor = (tab: WorkbenchSurface): TabSignal | null => {
    if (tab.kind !== "terminal") return null;
    const id = tabTerminalId(tab.resourceId);
    if (actionTabs.get(id)?.running) return { kind: "running", label: "running" };
    if (tab.id === surface?.id) return null;
    const code = terminalActivity.exitCodeOf(id);
    if (code !== undefined) return { kind: "exited", label: code == null ? "ended" : `exit ${code}` };
    if (terminalActivity.isStreaming(id)) return { kind: "streaming", label: "live" };
    if (terminalActivity.isUnseen(id)) return { kind: "unseen", label: "new output" };
    return null;
  };
  const closeConfirmation = (
    <ConfirmDialog
      visible={closing != null}
      title={`Close ${closing?.title ?? "terminal"}?`}
      message="Its shell is still running. Closing the tab stops it and any command it is running."
      confirmLabel="Close terminal"
      onConfirm={() => {
        if (closing) discardTerminal(closing);
        setClosing(null);
      }}
      onCancel={() => setClosing(null)}
    />
  );

  const bottom = surfaceDefinition.placement === "bottom";
  const storedSize = bottom ? workbench.layout.dock.bottom : workbench.layout.dock.right;
  const baseSize = storedSize ?? surfaceDefinition.defaultSize;
  const clamp = useCallback(
    (value: number) => (bottom ? clampBottomHeight(value, windowHeight) : clampRightWidth(value, windowWidth, occupiedWidth)),
    [bottom, windowHeight, windowWidth, occupiedWidth],
  );
  // While dragging the size lives here; the workbench (and its persistence) hears about it on release.
  const [live, setLive] = useState<number | null>(null);
  const dragStart = useRef(baseSize);
  const size = clamp(live ?? baseSize);

  const commit = useCallback(
    (value: number | null) => {
      if (bottom) workbench.setBottomHeight(value);
      else workbench.setRightWidth(value);
    },
    [bottom, workbench],
  );
  const onStart = useCallback(() => {
    dragStart.current = size;
  }, [size]);
  // A right dock grows leftwards and a bottom dock upwards, so pointer travel is subtracted.
  const onDrag = useCallback((delta: number) => setLive(clamp(dragStart.current - delta)), [clamp]);
  const onEnd = useCallback(() => {
    setLive((value) => {
      if (value != null) commit(value);
      return null;
    });
  }, [commit]);
  const onReset = useCallback(() => {
    setLive(null);
    commit(null);
  }, [commit]);
  const onNudge = useCallback((delta: number) => commit(clamp(size - delta)), [clamp, commit, size]);

  const header = (compact: boolean) => (
    <View style={[styles.header, compact && styles.headerCompact, { borderBottomColor: tokens.border }]}>
      {shownTabs.length > 1 ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.tabs} contentContainerStyle={styles.tabsContent}>
          {shownTabs.map((tab, index) => (
            <DockTab
              key={tab.id}
              tab={tab}
              index={index}
              active={tab.id === surface?.id}
              signal={signalFor(tab)}
              onActivate={onActivateSurface}
              onClose={requestClose}
              onReorder={onReorderSurface}
            />
          ))}
        </ScrollView>
      ) : (
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
          {surface?.title ?? surfaceDefinition.title}
        </Text>
      )}
      {row ? (
        // Which session this panel is showing — it follows the focused pane.
        <Text style={[typeScale.monoMeta, styles.followChip, { color: tokens.ink3 }]} numberOfLines={1}>
          {shownTabs.length <= 1 ? formatCwd(row.cwd) : row.title || `session ${row.id.slice(0, 8)}`}
        </Text>
      ) : null}
      <View style={styles.headerSpacer} />
      <Pressable
        onPress={onClose}
        accessibilityRole="button"
        accessibilityLabel={`Close ${surfaceDefinition.title.toLowerCase()} dock`}
        style={styles.closeButton}
      >
        <X size={14} strokeWidth={1.75} color={tokens.ink3} />
      </Pressable>
    </View>
  );

  if (!open) return null;

  if (bottom) {
    return (
      <View style={[styles.bottomDock, { height: size, borderTopColor: tokens.border, backgroundColor: tokens.bg0 }]}>
        <ResizeHandle
          orientation="horizontal"
          overlay="start"
          label={`Resize ${surfaceDefinition.title.toLowerCase()} dock`}
          onStart={onStart}
          onDrag={onDrag}
          onEnd={onEnd}
          onReset={onReset}
          onNudge={onNudge}
        />
        {header(true)}
        <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId, surface, openTerminal, closeFinishedTerminal })}</View>
        {closeConfirmation}
      </View>
    );
  }

  return (
    <View style={[styles.rightDock, { width: size, borderLeftColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <ResizeHandle
        orientation="vertical"
        overlay="start"
        label={`Resize ${surfaceDefinition.title.toLowerCase()} dock`}
        onStart={onStart}
        onDrag={onDrag}
        onEnd={onEnd}
        onReset={onReset}
        onNudge={onNudge}
      />
      {header(false)}
      <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId, surface, openTerminal, closeFinishedTerminal })}</View>
    </View>
  );
}


const styles = StyleSheet.create({
  rightDock: { flexShrink: 0, borderLeftWidth: StyleSheet.hairlineWidth },
  bottomDock: { flexShrink: 0, borderTopWidth: StyleSheet.hairlineWidth },
  header: {
    height: 38,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
  },
  headerCompact: { height: 30 },
  tabs: { flexGrow: 0, flexShrink: 1, alignSelf: "stretch" },
  tabsContent: { alignItems: "stretch" },
  tab: {
    flexDirection: "row",
    alignItems: "center",
    maxWidth: 170,
    paddingLeft: space.space8,
    borderBottomWidth: 2,
  },
  tabLabel: { flexShrink: 1, flexDirection: "row", alignItems: "center", alignSelf: "stretch" },
  runningDot: { marginLeft: space.space8, alignSelf: "center" },
  unseenDot: { width: 6, height: 6, borderRadius: 3, marginLeft: space.space8, alignSelf: "center" },
  liveBadge: { marginLeft: space.space8, paddingHorizontal: 4, borderRadius: 2, overflow: "hidden", alignSelf: "center" },
  exitLabel: { marginLeft: space.space8, flexShrink: 0 },
  tabClose: { width: 20, height: 20, marginLeft: 2, alignItems: "center", justifyContent: "center" },
  headerSpacer: { flex: 1 },
  followChip: { flexShrink: 1, maxWidth: 140 },
  closeButton: { width: 26, height: 26, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
});
