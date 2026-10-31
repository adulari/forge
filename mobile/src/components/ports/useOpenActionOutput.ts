import { useEffect } from "react";

import type { WorkbenchController } from "../workbench/WorkbenchProvider";
import { subscribeOpenActionOutput } from "./actionOutputBus";

/** Opens the terminal tab of an action that was just started, wherever the Run was pressed. */
export function useOpenActionOutput(workbench: WorkbenchController, enabled: boolean): void {
  const { openSurface } = workbench;
  useEffect(() => {
    if (!enabled) return;
    return subscribeOpenActionOutput((request) =>
      openSurface({ kind: "terminal", sessionId: request.sessionId, resourceId: request.terminalId, title: request.title }),
    );
  }, [enabled, openSurface]);
}
