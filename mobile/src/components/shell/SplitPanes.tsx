// Machined desktop shell — several sessions on one screen (D Split + Usage, docs/design/machined
// INVENTORY.md L196-260), up to four: two or three columns, or a 2x2 grid. Each pane carries a
// 38px mini-header (status dot · title · project · maximize / swap / approve / close) and its
// own composer.
//
// State model, and why pane 0 is not stored: pane 0 IS the routed session. The grid wraps the
// live `<Stack>` rather than replacing it, so navigation, deep links, the session shell's own
// tabs and every route outside /session keep working untouched. Only the *secondary* panes are
// state (components/workbench/layoutModel.ts); each mounts its own `SessionProvider`, giving it a
// real second socket, transcript and composer instead of a read-only mirror.
//
// Panes are absolutely positioned from `paneGeometry`, not nested flex rows, so pane 0 keeps one
// place in the React tree through every layout change and the router never remounts.
//
// Desktop-only chrome: the root layout renders this only for a paired expanded window, so on
// compact/medium (phone, narrow web) nothing here exists.
import { router } from "expo-router";
import { ArrowLeftRight, Check, Maximize2, Minimize2, SquareSplitHorizontal, X } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View, type LayoutChangeEvent } from "react-native";

import { SessionChat } from "../../app/session/[id]/index";
import { useSessions } from "../../lib/queries";
import { useShortcutLabel } from "../../lib/shortcuts";
import { SessionProvider, useSessionCtx } from "../../lib/sessionContext";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space, statusDotColor, type StatusDotState } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import {
  DEFAULT_WEIGHTS,
  MIN_ROW_SHARE,
  clampFraction,
  minWeight,
  normalizeWeights,
  paneGeometry,
  resizeBoundary,
  type PaneRect,
  type PaneSeam,
  type PaneWeights,
} from "../workbench/layoutModel";
import { useWorkbench } from "../workbench/WorkbenchProvider";
import { useSessionRow } from "./activeSession";
import { ResizeHandle } from "./ResizeHandle";
import { useDragSource, useDragging, useDropTarget, type DragPayload } from "./sessionDrag";

const HEADER_HEIGHT = 38;
const NEW_PANE_ZONE_WIDTH = 120;

// ---------------------------------------------------------------------------
// Mini-header
// ---------------------------------------------------------------------------

function HeaderAction({
  icon,
  onPress,
  disabled,
  accessibilityLabel,
  accessibilityHint,
}: {
  icon: React.ReactNode;
  onPress: () => void;
  disabled?: boolean;
  accessibilityLabel: string;
  accessibilityHint?: string;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={accessibilityHint}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.headerAction,
        disabled && styles.headerActionDisabled,
        (pressed || hovered) && { backgroundColor: hexToRgba(tokens.accent, pressed ? 0.18 : 0.1) },
      ]}
    >
      {icon}
    </Pressable>
  );
}

interface PaneActions {
  /** Pane index for drag-to-swap; null while there is nothing to swap with. */
  dragIndex: number | null;
  focused: boolean;
  maximized: boolean;
  /** Swapping needs a second pane to swap with. */
  canSwap: boolean;
  hiddenNote: string | null;
  onMaximize: () => void;
  onSwap: () => void;
  onClose: () => void;
}

function PaneHeader({
  title,
  project,
  state,
  canApprove,
  onApprove,
  actions,
}: {
  title: string;
  project: string | null;
  state: StatusDotState;
  canApprove: boolean;
  onApprove: () => void;
  actions: PaneActions;
}) {
  const tokens = useTokens();
  const maximizeHint = useShortcutLabel("workbench.maximize");
  const headerRef = useRef<View>(null);
  // Grab a pane by its header and drop it on another pane to swap them.
  useDragSource(headerRef, actions.dragIndex != null ? { kind: "pane", index: actions.dragIndex } : null);
  return (
    <View ref={headerRef} style={[styles.header, { borderBottomColor: tokens.border }]}>
      {actions.focused ? <View pointerEvents="none" style={[styles.focusBar, { backgroundColor: tokens.accent }]} /> : null}
      <View style={[styles.dot, { backgroundColor: statusDotColor(state, tokens) }]} />
      <Text style={[typeScale.bodyBold, styles.headerTitle, { color: tokens.ink }]} numberOfLines={1}>
        {title}
      </Text>
      {project ? (
        <Text style={[typeScale.monoMeta, styles.headerProject, { color: tokens.ink3 }]} numberOfLines={1}>
          {project}
        </Text>
      ) : null}
      <View style={styles.headerSpacer} />
      {actions.hiddenNote ? (
        <Text style={[typeScale.monoMeta, { color: tokens.warn }]} numberOfLines={1}>
          {actions.hiddenNote}
        </Text>
      ) : null}
      <HeaderAction
        icon={
          actions.maximized ? (
            <Minimize2 size={13} strokeWidth={1.75} color={tokens.ink3} />
          ) : (
            <Maximize2 size={13} strokeWidth={1.75} color={tokens.ink3} />
          )
        }
        onPress={actions.onMaximize}
        accessibilityLabel={actions.maximized ? "Restore pane" : "Maximize pane"}
        accessibilityHint={maximizeHint}
      />
      {actions.canSwap ? (
        <HeaderAction
          icon={<ArrowLeftRight size={13} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={actions.onSwap}
          accessibilityLabel="Swap panes"
        />
      ) : null}
      <HeaderAction
        icon={<Check size={13} strokeWidth={1.75} color={canApprove ? tokens.accent : tokens.ink4} />}
        onPress={onApprove}
        disabled={!canApprove}
        accessibilityLabel="Approve the pending decision"
      />
      <HeaderAction
        icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />}
        onPress={actions.onClose}
        accessibilityLabel="Close pane"
      />
    </View>
  );
}

/** Pane 0's header, driven off the fleet row (the routed session's own socket lives below,
 * inside the route — reaching it from here would mean opening a second one for the same id). */
function PrimaryPaneHeader({ sessionId, actions }: { sessionId: string; actions: PaneActions }) {
  const row = useSessionRow(sessionId);
  const state: StatusDotState = row?.waiting ? "waiting" : row?.busy ? "busy" : "idle";
  return (
    <PaneHeader
      title={row?.title || `session ${sessionId.slice(0, 8)}`}
      project={row ? formatCwd(row.cwd) : null}
      state={state}
      canApprove={!!row?.waiting}
      // The decision itself is answered in the pane below (the permission card is right there)
      // or in Inbox — which is exactly what `desktopMenu`'s own `session:approve` default does
      // when no session claims it.
      onApprove={() => router.push("/inbox")}
      actions={actions}
    />
  );
}

// ---------------------------------------------------------------------------
// Secondary panes
// ---------------------------------------------------------------------------

function SecondaryPaneBody({ sessionId, actions }: { sessionId: string; actions: PaneActions }) {
  const { snapshot, send } = useSessionCtx();
  const row = useSessionRow(sessionId);
  const pending = snapshot?.permission_prompt != null || snapshot?.question != null;
  const state: StatusDotState = pending ? "waiting" : (snapshot?.busy ?? row?.busy) ? "busy" : "idle";

  return (
    <>
      <PaneHeader
        title={snapshot?.title || row?.title || `session ${sessionId.slice(0, 8)}`}
        project={snapshot ? formatCwd(snapshot.cwd) : row ? formatCwd(row.cwd) : null}
        state={state}
        canApprove={pending && snapshot != null}
        onApprove={() => {
          if (snapshot) send({ kind: "allow", yes: true, seq: snapshot.prompt_seq });
        }}
        actions={actions}
      />
      <View style={styles.paneBody}>
        <SessionChat />
      </View>
    </>
  );
}

/** An empty slot: explains itself and lists the sessions that are not on screen yet. */
function PanePicker({ index, actions }: { index: number; actions: PaneActions }) {
  const tokens = useTokens();
  const workbench = useWorkbench();
  const { data: sessions } = useSessions();
  const open = new Set(workbench.paneIds.filter((id): id is string => id != null));
  const available = (sessions ?? []).filter((row) => !open.has(row.id));
  return (
    <>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        {actions.focused ? <View pointerEvents="none" style={[styles.focusBar, { backgroundColor: tokens.accent }]} /> : null}
        <Text style={[typeScale.bodyBold, styles.headerTitle, { color: tokens.ink2 }]}>Empty pane</Text>
        <View style={styles.headerSpacer} />
        <HeaderAction
          icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={actions.onClose}
          accessibilityLabel="Close pane"
        />
      </View>
      <View style={styles.paneBody}>
        {available.length === 0 ? (
          <EmptyState
            icon={SquareSplitHorizontal}
            message="Every session is already on screen. Start another and it opens here."
            action={<Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />}
          />
        ) : (
          <ScrollView contentContainerStyle={styles.picker}>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              Pick a session for this pane, or drag one in from the sidebar.
            </Text>
            {available.map((row) => (
              <Pressable
                key={row.id}
                onPress={() => workbench.openSession(row.id, { pane: index })}
                accessibilityRole="button"
                accessibilityLabel={`Open ${row.title || `session ${row.id.slice(0, 8)}`} in this pane`}
                style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
                  styles.pickerRow,
                  { borderColor: tokens.border, backgroundColor: hovered || pressed ? tokens.bg3 : tokens.bg2 },
                ]}
              >
                <View style={[styles.dot, { backgroundColor: statusDotColor(row.waiting ? "waiting" : row.busy ? "busy" : "idle", tokens) }]} />
                <Text style={[typeScale.body, styles.headerTitle, { color: tokens.ink }]} numberOfLines={1}>
                  {row.title || `session ${row.id.slice(0, 8)}`}
                </Text>
                <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
                  {formatCwd(row.cwd)}
                </Text>
              </Pressable>
            ))}
            <Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />
          </ScrollView>
        )}
      </View>
    </>
  );
}

function SecondaryPane({ sessionId, index, actions }: { sessionId: string | null; index: number; actions: PaneActions }) {
  if (!sessionId) return <PanePicker index={index} actions={actions} />;
  return (
    // Keyed so switching the session tears the provider (and its socket) down
    // instead of handing a live provider a different id.
    <SessionProvider key={sessionId} sessionId={sessionId}>
      <SecondaryPaneBody sessionId={sessionId} actions={actions} />
    </SessionProvider>
  );
}

// ---------------------------------------------------------------------------
// Drop targets (drag a session from the sidebar)
// ---------------------------------------------------------------------------

function DropZone({
  label,
  accepts,
  onDrop,
  style,
}: {
  label: string;
  accepts: (payload: DragPayload) => boolean;
  onDrop: (payload: DragPayload) => void;
  style?: object;
}) {
  const tokens = useTokens();
  const ref = useRef<View>(null);
  const [over, setOver] = useState(false);
  useDropTarget(ref, accepts, onDrop, setOver);
  return (
    <View
      ref={ref}
      style={[
        styles.dropZone,
        style,
        {
          borderColor: tokens.accent,
          backgroundColor: hexToRgba(tokens.accent, over ? 0.22 : 0.08),
        },
      ]}
    >
      <Text pointerEvents="none" style={[typeScale.bodyBold, { color: tokens.accent }]}>
        {label}
      </Text>
    </View>
  );
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

export interface PaneGridProps {
  /** The live route stack — pane 0's body. */
  primary: React.ReactNode;
}

function rectStyle(rect: PaneRect | null) {
  if (!rect) return styles.hiddenPane;
  return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
}

export function PaneGrid({ primary }: PaneGridProps) {
  const tokens = useTokens();
  const workbench = useWorkbench();
  const { layout, primaryId } = workbench;
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [live, setLive] = useState<PaneWeights | null>(null);
  const liveRef = useRef<PaneWeights | null>(null);
  const startRef = useRef<PaneWeights>(layout.weights);
  const dragging = useDragging();

  // Without a routed session (Inbox, Settings, ...) there is nothing to put next to: show the
  // route alone. The secondary panes keep their state and return with the next session.
  const active = primaryId != null && layout.secondary.length > 0;
  const total = active ? 1 + layout.secondary.length : 1;
  const weights = live ?? layout.weights;

  const onLayout = useCallback((event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    setSize((current) => (current && current.width === width && current.height === height ? current : { width, height }));
  }, []);

  const geometry = useMemo(
    () => (size ? paneGeometry(total, size.width, size.height, weights, active ? layout.maximized : null) : null),
    [size, total, weights, layout.maximized, active],
  );

  const updateLive = useCallback((next: PaneWeights | null) => {
    liveRef.current = next;
    setLive(next);
  }, []);

  const seamStart = useCallback(() => {
    startRef.current = layout.weights;
  }, [layout.weights]);

  const seamDrag = useCallback(
    (seam: PaneSeam, delta: number, visible: number) => {
      if (!size) return;
      const start = startRef.current;
      if (seam.key === "grid") {
        const grid = [...normalizeWeights(start.grid, 2)];
        if (seam.orientation === "vertical") grid[0] = clampFraction((start.grid[0] ?? 0.5) + delta / size.width, minWeight(size.width, 2));
        else grid[1] = clampFraction((start.grid[1] ?? 0.5) + delta / size.height, MIN_ROW_SHARE);
        // Grid weights are two independent shares, so they must not be re-normalised to sum to 1.
        updateLive({ ...start, grid: seam.orientation === "vertical" ? [grid[0] ?? 0.5, start.grid[1] ?? 0.5] : [start.grid[0] ?? 0.5, grid[1] ?? 0.5] });
        return;
      }
      const shares = normalizeWeights(start[seam.key], visible);
      updateLive({ ...start, [seam.key]: resizeBoundary(shares, seam.boundary, delta / size.width, minWeight(size.width, visible)) });
    },
    [size, updateLive],
  );

  const seamEnd = useCallback(
    (seam: PaneSeam) => {
      const next = liveRef.current;
      if (next) workbench.setPaneWeights(seam.key, next[seam.key]);
      updateLive(null);
    },
    [updateLive, workbench],
  );

  const seamReset = useCallback(
    (seam: PaneSeam) => {
      updateLive(null);
      workbench.setPaneWeights(seam.key, [...DEFAULT_WEIGHTS[seam.key]]);
    },
    [updateLive, workbench],
  );

  const seamNudge = useCallback(
    (seam: PaneSeam, delta: number, visible: number) => {
      if (!size) return;
      const current = layout.weights;
      if (seam.key === "grid") {
        const grid = [...current.grid];
        if (seam.orientation === "vertical") grid[0] = clampFraction((grid[0] ?? 0.5) + delta / size.width, minWeight(size.width, 2));
        else grid[1] = clampFraction((grid[1] ?? 0.5) + delta / size.height, MIN_ROW_SHARE);
        workbench.setPaneWeights("grid", grid);
        return;
      }
      const shares = normalizeWeights(current[seam.key], visible);
      workbench.setPaneWeights(seam.key, resizeBoundary(shares, seam.boundary, delta / size.width, minWeight(size.width, visible)));
    },
    [layout.weights, size, workbench],
  );

  const visible = geometry?.visible ?? 1;
  // Focus must never rest on a pane the window is too narrow to show.
  const { focusPane } = workbench;
  useEffect(() => {
    if (active && layout.maximized == null && geometry && layout.focus >= visible) focusPane(visible - 1);
  }, [active, geometry, layout.focus, layout.maximized, visible, focusPane]);
  const hidden = active && layout.maximized == null ? total - visible : 0;
  const multi = active && (geometry?.rects.filter(Boolean).length ?? 0) > 1;

  const actionsFor = (index: number): PaneActions => ({
    focused: multi && layout.focus === index,
    maximized: layout.maximized === index,
    canSwap: active && layout.maximized == null,
    hiddenNote: hidden > 0 && index === visible - 1 ? `+${hidden} hidden` : null,
    onMaximize: () => workbench.toggleMaximize(index),
    onSwap: () => workbench.swapWithPrimary(Math.max(1, index)),
    dragIndex: multi ? index : null,
    onClose: () => workbench.closePane(index),
  });

  // A drop on pane `index`: a session opens there, a dragged pane swaps places with it.
  const dropOn = (index: number) => (payload: DragPayload) => {
    if (payload.kind === "session") workbench.openSession(payload.id, { pane: index });
    else if (payload.kind === "pane" && payload.index !== index) {
      if (payload.index === 0 || index === 0) workbench.swapWithPrimary(Math.max(payload.index, index));
      else workbench.swapPanes(payload.index, index);
    }
  };
  const acceptsOn = (index: number) => (payload: DragPayload) =>
    payload.kind === "session" || (payload.kind === "pane" && payload.index !== index);
  const dropLabel = dragging?.kind === "pane" ? "Swap here" : "Open here";

  const focusOn = (index: number) => () => {
    workbench.focusPane(index);
    return false;
  };

  const showSplitZone = dragging?.kind === "session" && total < 4;

  return (
    <View style={styles.container} onLayout={onLayout}>
      <View
        style={[styles.pane, geometry ? rectStyle(geometry.rects[0] ?? null) : styles.fillPane]}
        onStartShouldSetResponderCapture={focusOn(0)}
      >
        {active && primaryId ? <PrimaryPaneHeader sessionId={primaryId} actions={actionsFor(0)} /> : null}
        <View style={styles.paneBody}>{primary}</View>
        {dragging != null && dragging.kind !== "tab" && geometry?.rects[0] && !(dragging.kind === "pane" && dragging.index === 0) ? (
          <DropZone label={dropLabel} accepts={acceptsOn(0)} onDrop={dropOn(0)} style={styles.paneDrop} />
        ) : null}
      </View>

      {active
        ? layout.secondary.map((slot, i) => {
            const index = i + 1;
            const rect = geometry?.rects[index] ?? null;
            return (
              <View
                key={`pane-${index}`}
                style={[styles.pane, rectStyle(rect), { borderLeftColor: tokens.border }]}
                onStartShouldSetResponderCapture={focusOn(index)}
              >
                <SecondaryPane sessionId={slot} index={index} actions={actionsFor(index)} />
                {dragging != null && dragging.kind !== "tab" && rect && !(dragging.kind === "pane" && dragging.index === index) ? (
                  <DropZone label={dropLabel} accepts={acceptsOn(index)} onDrop={dropOn(index)} style={styles.paneDrop} />
                ) : null}
              </View>
            );
          })
        : null}

      {geometry?.seams.map((seam) => (
        <ResizeHandle
          key={`${seam.key}-${seam.orientation}-${seam.boundary}`}
          orientation={seam.orientation}
          label={seam.orientation === "vertical" ? "Resize split panes" : "Resize pane rows"}
          onStart={seamStart}
          onDrag={(delta) => seamDrag(seam, delta, visible)}
          onEnd={() => seamEnd(seam)}
          onReset={() => seamReset(seam)}
          onNudge={(delta) => seamNudge(seam, delta, visible)}
          style={
            seam.orientation === "vertical"
              ? { position: "absolute", left: seam.at - 4, top: 0, bottom: 0, zIndex: 20 }
              : { position: "absolute", top: seam.at - 4, left: 0, right: 0, zIndex: 20 }
          }
        />
      ))}

      {showSplitZone && layout.maximized == null ? (
        <DropZone
          label="New pane"
          accepts={(payload) => payload.kind === "session"}
          onDrop={(payload) => payload.kind === "session" && workbench.openSession(payload.id, "new")}
          style={styles.newPaneZone}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, minHeight: 0, minWidth: 0 },
  pane: { position: "absolute", minWidth: 0, flexDirection: "column", overflow: "hidden" },
  fillPane: { left: 0, top: 0, right: 0, bottom: 0 },
  hiddenPane: { left: 0, top: 0, width: 0, height: 0, display: "none" },
  paneBody: { flex: 1, minHeight: 0 },
  header: {
    height: HEADER_HEIGHT,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
  },
  focusBar: { position: "absolute", top: 0, left: 0, right: 0, height: 2 },
  headerTitle: { flexShrink: 1 },
  headerProject: { flexShrink: 2 },
  headerSpacer: { flex: 1 },
  headerAction: { width: 26, height: 26, borderRadius: 3, alignItems: "center", justifyContent: "center" },
  headerActionDisabled: { opacity: 0.5 },
  dot: { width: 6, height: 6, borderRadius: 3 },
  picker: { padding: space.space16, gap: space.space8 },
  pickerRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    height: 36,
    paddingHorizontal: space.space12,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
  },
  dropZone: {
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 2,
    borderStyle: "dashed",
    borderRadius: radii.radius8,
  },
  paneDrop: { position: "absolute", left: 8, top: 8, right: 8, bottom: 8, zIndex: 30 },
  newPaneZone: { position: "absolute", right: 8, top: 8, bottom: 8, width: NEW_PANE_ZONE_WIDTH, zIndex: 40 },
});

