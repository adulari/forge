// Whether IDE mode is on screen, and how to switch. One hook so the title bar, the shortcut, the
// menu and the settings row all agree.
import { router, usePathname } from "expo-router";
import { useCallback } from "react";

import { useAuth } from "../../lib/auth";
import { setIdeModeEnabled, useIdeModePreferences } from "../../lib/ideModePrefs";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { isIdeRoute } from "./ideRoutes";

export type ShellMode = "chat" | "ide";

export interface IdeModeState {
  /** The window can host IDE mode at all: paired and at the expanded breakpoint. */
  available: boolean;
  /** The preference. */
  enabled: boolean;
  /** IDE mode is what the window shows right now. */
  visible: boolean;
  mode: ShellMode;
  setMode: (mode: ShellMode) => void;
  toggle: () => void;
}

export function useIdeMode(): IdeModeState {
  const { isPaired } = useAuth();
  const { isExpanded } = useBreakpoint();
  const pathname = usePathname();
  const { enabled } = useIdeModePreferences();
  const available = isPaired && isExpanded;
  const visible = available && enabled && isIdeRoute(pathname);
  const onIdeRoute = isIdeRoute(pathname);

  const setMode = useCallback(
    (mode: ShellMode) => {
      if (mode === "chat") {
        setIdeModeEnabled(false);
        return;
      }
      setIdeModeEnabled(true);
      // Coming back from settings or another full page: land on the workbench, not the old page.
      if (!onIdeRoute) router.replace("/");
    },
    [onIdeRoute],
  );
  const toggle = useCallback(() => setMode(visible ? "chat" : "ide"), [setMode, visible]);

  return { available, enabled, visible, mode: visible ? "ide" : "chat", setMode, toggle };
}
