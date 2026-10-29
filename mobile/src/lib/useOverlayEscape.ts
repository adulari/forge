import { useEffect, useRef } from "react";
import { Platform } from "react-native";

import { registerOverlay } from "./overlayEscapeStack";

/** Registers while `active`; the latest `close` is always the one called. No-op off web. */
export function useOverlayEscape(active: boolean, close: () => void): void {
  const closeRef = useRef(close);
  useEffect(() => {
    closeRef.current = close;
  });
  useEffect(() => {
    if (!active || Platform.OS !== "web") return;
    return registerOverlay(() => closeRef.current());
  }, [active]);
}
