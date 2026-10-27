// Machined desktop shell — the dock container (D Split + Usage / D Rail + Terminal / D Git
// Review, docs/design/machined INVENTORY.md): a typed registry of docked panels, each declaring
// where it attaches. Right-edge docks share one resizable column beside the content, switched
// with tabs; the terminal is a resizable bottom strip inside the content column. Sizes are
// per dock kind, clamped to the window, persisted by the workbench, and reset by double-click.
//
// Adding a dock is one registry entry plus the hotkey/menu wiring in app/_layout.tsx — nothing
// else in this file changes.
import { PanelRight, X } from "lucide-react-native";
import React, { useCallback, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import { GitReviewDock } from "../git/GitReviewDock";
import { BrowserPreviewDock } from "../preview/BrowserPreviewDock";
import { EmptyState } from "../ds/EmptyState";
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
import { terminalTitle } from "./terminalModel";
import { UsageDock } from "./UsageDock";

export type DockKind = WorkbenchSurfaceKind;

function DockTab({
  tab,
  index,
  active,
  onActivate,
  onClose,
  onReorder,
}: {
  tab: WorkbenchSurface;
  index: number;
  active: boolean;
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
      <WorkspaceDock sessionId={sessionId} resourceId={surface?.resourceId ?? null} />
    ),
  },
  preview: {
    render: ({ sessionId, surface }) =>
      sessionId && surface ? <BrowserPreviewDock sessionId={sessionId} surface={surface} /> : <NeedsSession what="its preview" />,
  },
  terminal: {
    render: ({ sessionId, surface, openTerminal }) => (
      <TerminalDock
        sessionId={sessionId}
        terminalId={surface?.resourceId ?? "term-1"}
        onOpenTerminal={openTerminal}
      />
    ),
  },
  tasks: { render: ({ sessionId }) => <SessionDock kind="tasks" sessionId={sessionId} /> },
  agents: { render: ({ sessionId }) => <SessionDock kind="agents" sessionId={sessionId} /> },
  plan: { render: ({ sessionId }) => <SessionDock kind="plan" sessionId={sessionId} /> },
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
        title: terminalTitle(terminalId),
      });
    },
    [effectiveSessionId, workbench],
  );
  const row = useSessionRow(surfaceDefinition.sessionScoped ? effectiveSessionId : null);

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
      {tabs.length > 1 ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.tabs} contentContainerStyle={styles.tabsContent}>
          {tabs.map((tab, index) => (
            <DockTab
              key={tab.id}
              tab={tab}
              index={index}
              active={tab.id === surface?.id}
              onActivate={onActivateSurface}
              onClose={onCloseSurface}
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
          {tabs.length <= 1 ? formatCwd(row.cwd) : row.title || `session ${row.id.slice(0, 8)}`}
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
        <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId, surface, openTerminal })}</View>
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
      <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId, surface, openTerminal })}</View>
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
  tabLabel: { flexShrink: 1, justifyContent: "center", alignSelf: "stretch" },
  tabClose: { width: 20, height: 20, marginLeft: 2, alignItems: "center", justifyContent: "center" },
  headerSpacer: { flex: 1 },
  followChip: { flexShrink: 1, maxWidth: 140 },
  closeButton: { width: 26, height: 26, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
});
