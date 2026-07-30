// Machined desktop shell — the dock container (D Split + Usage / D Rail + Terminal / D Git
// Review, docs/design/machined INVENTORY.md): a typed registry of docked panels, each declaring
// where it attaches. Right-edge docks (usage, git review) are fixed-width columns beside the
// content; the terminal is a resizable bottom strip inside the content column.
//
// Adding a dock is one registry entry plus the hotkey/menu wiring in app/_layout.tsx — nothing
// else in this file changes.
import { X } from "lucide-react-native";
import React, { useCallback, useRef, useState } from "react";
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  type GestureResponderEvent,
} from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import {
  WORKBENCH_SURFACE_DEFINITIONS,
  type WorkbenchSurface,
  type WorkbenchSurfaceKind,
} from "../workbench/model";
import { GitReviewDock } from "../git/GitReviewDock";
import { useSessionRow } from "./activeSession";
import { TerminalDock } from "./TerminalDock";
import { UsageDock } from "./UsageDock";

export type DockKind = WorkbenchSurfaceKind;

export interface DockContext {
  sessionId: string | null;
}

interface DockDefinition {
  render: (ctx: DockContext) => React.ReactNode;
}

const DOCK_REGISTRY: Record<DockKind, DockDefinition> = {
  usage: {
    render: () => <UsageDock />,
  },
  git: {
    // The dock follows the active routed session unless a future resource tab pins one.
    render: ({ sessionId }) => (sessionId ? <GitReviewDock sessionId={sessionId} /> : null),
  },
  terminal: {
    render: ({ sessionId }) => <TerminalDock sessionId={sessionId} />,
  },
};

const BOTTOM_MIN_HEIGHT = 110;
const BOTTOM_MAX_HEIGHT = 560;

export interface DockHostProps {
  open: boolean;
  dock?: DockKind;
  /** Concrete tab selected by the workbench. `dock` remains as a compact fallback for callers. */
  surface?: WorkbenchSurface | null;
  /** Other open tabs in this placement. They remain mounted in the workbench model when hidden. */
  tabs?: readonly WorkbenchSurface[];
  /** Session the dock acts on — ignored by docks that aren't session-scoped. */
  sessionId?: string | null;
  onActivateSurface?: (id: string) => void;
  onClose: () => void;
}

export function DockHost({
  open,
  dock = "usage",
  surface = null,
  tabs = [],
  sessionId = null,
  onActivateSurface,
  onClose,
}: DockHostProps) {
  const tokens = useTokens();
  const kind = surface?.kind ?? dock;
  const definition = DOCK_REGISTRY[kind];
  const surfaceDefinition = WORKBENCH_SURFACE_DEFINITIONS[kind];
  const effectiveSessionId = surface?.sessionId ?? sessionId;
  const row = useSessionRow(surfaceDefinition.sessionScoped ? effectiveSessionId : null);
  const [height, setHeight] = useState(surfaceDefinition.defaultSize);
  // Drag bookkeeping lives entirely in handlers (never read during render): `height` mirrors
  // `drag.current.height` for layout, the ref is what the gesture math reads.
  const drag = useRef({
    startY: 0,
    startHeight: surfaceDefinition.defaultSize,
    height: surfaceDefinition.defaultSize,
  });

  const onGrantResize = useCallback((event: GestureResponderEvent) => {
    drag.current.startY = event.nativeEvent.pageY;
    drag.current.startHeight = drag.current.height;
  }, []);

  const onMoveResize = useCallback((event: GestureResponderEvent) => {
    const delta = drag.current.startY - event.nativeEvent.pageY;
    const next = Math.max(BOTTOM_MIN_HEIGHT, Math.min(BOTTOM_MAX_HEIGHT, drag.current.startHeight + delta));
    drag.current.height = next;
    setHeight(next);
  }, []);

  const header = useCallback(
    (compact: boolean) => (
      <View style={[styles.header, compact && styles.headerCompact, { borderBottomColor: tokens.border }]}>
        {tabs.length > 1 ? (
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            style={styles.tabs}
            contentContainerStyle={styles.tabsContent}
          >
            {tabs.map((tab) => {
              const active = tab.id === surface?.id;
              return (
                <Pressable
                  key={tab.id}
                  onPress={() => onActivateSurface?.(tab.id)}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: active }}
                  accessibilityLabel={`Open ${tab.title} surface`}
                  style={[
                    styles.tab,
                    {
                      borderBottomColor: active ? tokens.accent : "transparent",
                    },
                  ]}
                >
                  <Text
                    style={[
                      typeScale.bodyBold,
                      { color: active ? tokens.ink : tokens.ink3 },
                    ]}
                    numberOfLines={1}
                  >
                    {tab.title}
                  </Text>
                </Pressable>
              );
            })}
          </ScrollView>
        ) : (
          <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
            {surface?.title ?? surfaceDefinition.title}
          </Text>
        )}
        {row && tabs.length <= 1 ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
            {formatCwd(row.cwd)}
          </Text>
        ) : null}
        {tabs.length <= 1 ? <View style={styles.headerSpacer} /> : null}
        <Pressable
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={`Close ${surfaceDefinition.title.toLowerCase()} dock`}
          style={styles.closeButton}
        >
          <X size={14} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </View>
    ),
    [
      onActivateSurface,
      onClose,
      row,
      surface?.id,
      surface?.title,
      surfaceDefinition.title,
      tabs,
      tokens.accent,
      tokens.border,
      tokens.ink,
      tokens.ink3,
    ],
  );

  if (!open) return null;

  if (surfaceDefinition.placement === "bottom") {
    return (
      <View style={[styles.bottomDock, { height, borderTopColor: tokens.border, backgroundColor: tokens.bg0 }]}>
        <View
          onStartShouldSetResponder={() => true}
          onMoveShouldSetResponder={() => true}
          onResponderGrant={onGrantResize}
          onResponderMove={onMoveResize}
          style={styles.grip}
          accessibilityRole="adjustable"
          accessibilityLabel={`Resize ${surfaceDefinition.title.toLowerCase()} dock`}
        />
        {header(true)}
        <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId })}</View>
      </View>
    );
  }

  return (
    <View
      style={[
        styles.rightDock,
        {
          width: surfaceDefinition.defaultSize,
          borderLeftColor: tokens.border,
          backgroundColor: tokens.bg1,
        },
      ]}
    >
      {header(false)}
      <View style={styles.body}>{definition.render({ sessionId: effectiveSessionId })}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  rightDock: { flexShrink: 0, borderLeftWidth: StyleSheet.hairlineWidth },
  bottomDock: { flexShrink: 0, borderTopWidth: StyleSheet.hairlineWidth },
  // A 5px seam above the header is the whole drag target, so the dock's top hairline stays a
  // hairline instead of growing into a visible handle.
  grip: { height: 5 },
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
  tabs: { flex: 1, alignSelf: "stretch" },
  tabsContent: { alignItems: "stretch" },
  tab: {
    minWidth: 72,
    maxWidth: 150,
    justifyContent: "center",
    paddingHorizontal: space.space8,
    borderBottomWidth: 2,
  },
  headerSpacer: { flex: 1 },
  closeButton: { width: 26, height: 26, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
});
