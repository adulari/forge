// The one drag seam every resizable edge of the desktop workbench shares: between panes, on the
// left of a right dock, above the bottom dock. A 9px transparent hit area keeps the painted
// divider a true hairline; hovering, focusing or dragging thickens it to the accent colour so the
// edge reads as draggable before anyone tries. Double-click (or double-tap) resets, and the arrow
// keys nudge, so it is not mouse-only.
import React, { useCallback, useRef, useState } from "react";
import { Platform, StyleSheet, View, type GestureResponderEvent, type ViewStyle } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";

const HIT = 9;
const DOUBLE_PRESS_MS = 350;
const NUDGE = 16;
const NUDGE_BIG = 64;

export interface ResizeHandleProps {
  /** `vertical` is a vertical bar that resizes widths (drag along x); `horizontal` resizes heights. */
  orientation: "vertical" | "horizontal";
  label: string;
  /** Called on grant so the owner can snapshot its current size. */
  onStart?: () => void;
  /** Total pointer travel in px since the grant (x for vertical, y for horizontal). */
  onDrag: (delta: number) => void;
  onEnd?: () => void;
  /** Double-click: restore the default size. */
  onReset?: () => void;
  /** Keyboard nudge in px along the handle's axis. */
  onNudge?: (delta: number) => void;
  /** Float the hit area over the neighbouring edge instead of taking layout space. */
  overlay?: "start" | "end";
  style?: ViewStyle;
}

type WebHandlers = {
  onMouseEnter: () => void;
  onMouseLeave: () => void;
  onFocus: () => void;
  onBlur: () => void;
  onKeyDown: (event: { key: string; shiftKey: boolean; preventDefault: () => void }) => void;
};

export function ResizeHandle({ orientation, label, onStart, onDrag, onEnd, onReset, onNudge, overlay, style }: ResizeHandleProps) {
  const tokens = useTokens();
  const [hot, setHot] = useState(false);
  const [dragging, setDragging] = useState(false);
  const gesture = useRef({ origin: 0, lastGrant: 0 });
  const vertical = orientation === "vertical";

  const position = useCallback(
    (event: GestureResponderEvent) => (vertical ? event.nativeEvent.pageX : event.nativeEvent.pageY),
    [vertical],
  );

  const onGrant = useCallback(
    (event: GestureResponderEvent) => {
      const now = Date.now();
      if (now - gesture.current.lastGrant < DOUBLE_PRESS_MS) {
        gesture.current.lastGrant = 0;
        onReset?.();
        return;
      }
      gesture.current.lastGrant = now;
      gesture.current.origin = position(event);
      setDragging(true);
      onStart?.();
    },
    [onReset, onStart, position],
  );

  const onMove = useCallback(
    (event: GestureResponderEvent) => {
      if (!dragging && gesture.current.lastGrant === 0) return;
      onDrag(position(event) - gesture.current.origin);
    },
    [dragging, onDrag, position],
  );

  const onRelease = useCallback(() => {
    setDragging(false);
    onEnd?.();
  }, [onEnd]);

  const web: WebHandlers | null =
    Platform.OS === "web"
      ? {
          onMouseEnter: () => setHot(true),
          onMouseLeave: () => setHot(false),
          onFocus: () => setHot(true),
          onBlur: () => setHot(false),
          onKeyDown: (event) => {
            if (!onNudge) return;
            const step = event.shiftKey ? NUDGE_BIG : NUDGE;
            const keys = vertical ? { back: "ArrowLeft", forward: "ArrowRight" } : { back: "ArrowUp", forward: "ArrowDown" };
            if (event.key === keys.back) {
              event.preventDefault();
              onNudge(-step);
            } else if (event.key === keys.forward) {
              event.preventDefault();
              onNudge(step);
            } else if (event.key === "Enter" || event.key === "Home") {
              event.preventDefault();
              onReset?.();
            }
          },
        }
      : null;

  const active = hot || dragging;
  const thickness = active ? 2 : StyleSheet.hairlineWidth;
  const hit: ViewStyle = vertical ? { width: HIT, alignItems: "center" } : { height: HIT, justifyContent: "center" };
  const place: ViewStyle | null = overlay
    ? vertical
      ? { position: "absolute", top: 0, bottom: 0, zIndex: 20, [overlay === "start" ? "left" : "right"]: -Math.floor(HIT / 2) }
      : { position: "absolute", left: 0, right: 0, zIndex: 20, [overlay === "start" ? "top" : "bottom"]: -Math.floor(HIT / 2) }
    : null;

  return (
    <View
      onStartShouldSetResponder={() => true}
      onMoveShouldSetResponder={() => true}
      onResponderTerminationRequest={() => false}
      onResponderGrant={onGrant}
      onResponderMove={onMove}
      onResponderRelease={onRelease}
      onResponderTerminate={onRelease}
      focusable
      accessibilityRole="adjustable"
      accessibilityLabel={label}
      accessibilityHint={onReset ? "Drag to resize, double-click to reset, arrow keys nudge" : "Drag to resize"}
      style={[hit, place, style, Platform.OS === "web" && (vertical ? webColResize : webRowResize)]}
      {...(web as object | null)}
    >
      <View
        pointerEvents="none"
        style={[
          vertical ? { flex: 1, width: thickness } : { height: thickness, alignSelf: "stretch" },
          { backgroundColor: active ? tokens.accent : tokens.border },
        ]}
      />
    </View>
  );
}

// RN has no typed `cursor`; same untyped web passthrough Screen.tsx uses for `overscrollBehavior`.
const webColResize = { cursor: "col-resize" } as object as ViewStyle;
const webRowResize = { cursor: "row-resize" } as object as ViewStyle;
