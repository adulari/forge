// Drag a session out of the sidebar onto a pane, drag a pane header onto another pane to swap
// them, and drag dock tabs to reorder them. Web/desktop only: native HTML5 drag-and-drop attached
// to the underlying DOM nodes (react-native-web hands out the DOM element as the ref), because
// React Native has no drag-and-drop of its own. Every hook is a no-op on native, where none of
// this chrome renders anyway.
import { useEffect, useSyncExternalStore, type RefObject } from "react";
import type { View } from "react-native";

import { isWeb } from "../../lib/platform";

export type DragPayload =
  | { kind: "session"; id: string }
  | { kind: "pane"; index: number }
  | { kind: "tab"; index: number };

export const DRAG_MIME = "application/x-forge-drag";

let dragging: DragPayload | null = null;
const listeners = new Set<() => void>();

function setDragging(payload: DragPayload | null): void {
  if (dragging === payload) return;
  dragging = payload;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** What is being dragged right now, or null. Drop overlays only exist while this is set. */
export function useDragging(): DragPayload | null {
  return useSyncExternalStore(subscribe, () => dragging, () => null);
}

export function parsePayload(raw: string | undefined): DragPayload | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as Partial<DragPayload> & { id?: unknown; index?: unknown };
    if (value.kind === "session" && typeof value.id === "string") return { kind: "session", id: value.id };
    if ((value.kind === "pane" || value.kind === "tab") && typeof value.index === "number") {
      return { kind: value.kind, index: value.index };
    }
  } catch {
    // Not one of ours (a file or text drag): ignore it.
  }
  return null;
}

function domNode(ref: RefObject<View | null>): HTMLElement | null {
  const node = ref.current as unknown;
  return typeof HTMLElement !== "undefined" && node instanceof HTMLElement ? node : null;
}

export function useDragSource(ref: RefObject<View | null>, payload: DragPayload | null): void {
  const key = payload ? JSON.stringify(payload) : null;
  useEffect(() => {
    if (!isWeb || key == null) return;
    const node = domNode(ref);
    if (!node) return;
    node.setAttribute("draggable", "true");
    const onStart = (event: DragEvent) => {
      event.dataTransfer?.setData(DRAG_MIME, key);
      if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
      setDragging(JSON.parse(key) as DragPayload);
    };
    const onEnd = () => setDragging(null);
    node.addEventListener("dragstart", onStart);
    node.addEventListener("dragend", onEnd);
    return () => {
      node.removeAttribute("draggable");
      node.removeEventListener("dragstart", onStart);
      node.removeEventListener("dragend", onEnd);
    };
  }, [ref, key]);
}

export function useDropTarget(
  ref: RefObject<View | null>,
  accepts: (payload: DragPayload) => boolean,
  onDrop: (payload: DragPayload) => void,
  onHover?: (over: boolean) => void,
): void {
  useEffect(() => {
    if (!isWeb) return;
    const node = domNode(ref);
    if (!node) return;
    const allowed = (event: DragEvent) => {
      const current = dragging ?? parsePayload(event.dataTransfer?.getData(DRAG_MIME));
      return current != null && accepts(current);
    };
    const onOver = (event: DragEvent) => {
      if (!allowed(event)) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      onHover?.(true);
    };
    const onLeave = () => onHover?.(false);
    const onDropEvent = (event: DragEvent) => {
      const payload = parsePayload(event.dataTransfer?.getData(DRAG_MIME)) ?? dragging;
      if (!payload || !accepts(payload)) return;
      event.preventDefault();
      onHover?.(false);
      setDragging(null);
      onDrop(payload);
    };
    node.addEventListener("dragenter", onOver);
    node.addEventListener("dragover", onOver);
    node.addEventListener("dragleave", onLeave);
    node.addEventListener("drop", onDropEvent);
    return () => {
      node.removeEventListener("dragenter", onOver);
      node.removeEventListener("dragover", onOver);
      node.removeEventListener("dragleave", onLeave);
      node.removeEventListener("drop", onDropEvent);
    };
  }, [ref, accepts, onDrop, onHover]);
}
