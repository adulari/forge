// Machined desktop shell — the dock container (D Split + Usage / D Rail + Terminal / D Git
// Review, docs/design/machined INVENTORY.md): a typed registry of docked panels, each declaring
// where it attaches. Right-edge docks (usage, git review) are fixed-width columns beside the
// content; the terminal is a resizable bottom strip inside the content column.
//
// Adding a dock is one registry entry plus the hotkey/menu wiring in app/_layout.tsx — nothing
// else in this file changes.
import { X } from "lucide-react-native";
import React, { useCallback, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, type GestureResponderEvent } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import { GitReviewDock } from "../git/GitReviewDock";
import { useSessionRow } from "./activeSession";
import { TerminalDock } from "./TerminalDock";
import { UsageDock } from "./UsageDock";

export type DockKind = "usage" | "terminal" | "git";
export type DockPlacement = "right" | "bottom";

export interface DockContext {
  sessionId: string | null;
}

interface DockDefinition {
  title: string;
  placement: DockPlacement;
  /** Width for a right dock, resting height for a bottom one. */
  size: number;
  /** Docks scoped to one session show its project in the header and need an id to open. */
  sessionScoped: boolean;
  render: (ctx: DockContext) => React.ReactNode;
}

const DOCK_REGISTRY: Record<DockKind, DockDefinition> = {
  usage: {
    title: "Usage",
    placement: "right",
    size: 288,
    sessionScoped: false,
    render: () => <UsageDock />,
  },
  git: {
    title: "Git review",
    placement: "right",
    size: 420,
    sessionScoped: true,
    // GitReviewDock is a parallel builder's component; the contract is `{ sessionId: string }`.
    render: ({ sessionId }) => (sessionId ? <GitReviewDock sessionId={sessionId} /> : null),
  },
  terminal: {
    title: "Terminal",
    placement: "bottom",
    size: 190,
    sessionScoped: true,
    render: ({ sessionId }) => <TerminalDock sessionId={sessionId} />,
  },
};

const BOTTOM_MIN_HEIGHT = 110;
const BOTTOM_MAX_HEIGHT = 560;

export interface DockHostProps {
  open: boolean;
  dock?: DockKind;
  /** Session the dock acts on — ignored by docks that aren't session-scoped. */
  sessionId?: string | null;
  onClose: () => void;
}

export function DockHost({ open, dock = "usage", sessionId = null, onClose }: DockHostProps) {
  const tokens = useTokens();
  const definition = DOCK_REGISTRY[dock];
  const row = useSessionRow(definition.sessionScoped ? sessionId : null);
  const [height, setHeight] = useState(definition.size);
  // Drag bookkeeping lives entirely in handlers (never read during render): `height` mirrors
  // `drag.current.height` for layout, the ref is what the gesture math reads.
  const drag = useRef({ startY: 0, startHeight: definition.size, height: definition.size });

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
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
          {definition.title}
        </Text>
        {row ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
            {formatCwd(row.cwd)}
          </Text>
        ) : null}
        <View style={styles.headerSpacer} />
        <Pressable
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={`Close ${definition.title.toLowerCase()} dock`}
          style={styles.closeButton}
        >
          <X size={14} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </View>
    ),
    [definition.title, onClose, row, tokens.border, tokens.ink, tokens.ink3],
  );

  if (!open) return null;

  if (definition.placement === "bottom") {
    return (
      <View style={[styles.bottomDock, { height, borderTopColor: tokens.border, backgroundColor: tokens.bg0 }]}>
        <View
          onStartShouldSetResponder={() => true}
          onMoveShouldSetResponder={() => true}
          onResponderGrant={onGrantResize}
          onResponderMove={onMoveResize}
          style={styles.grip}
          accessibilityRole="adjustable"
          accessibilityLabel={`Resize ${definition.title.toLowerCase()} dock`}
        />
        {header(true)}
        <View style={styles.body}>{definition.render({ sessionId })}</View>
      </View>
    );
  }

  return (
    <View
      style={[styles.rightDock, { width: definition.size, borderLeftColor: tokens.border, backgroundColor: tokens.bg1 }]}
    >
      {header(false)}
      <View style={styles.body}>{definition.render({ sessionId })}</View>
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
  headerSpacer: { flex: 1 },
  closeButton: { width: 26, height: 26, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
});
