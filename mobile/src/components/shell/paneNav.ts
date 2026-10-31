// How a session screen opens another session or its agents list. Pane 0 is the routed Stack and
// navigates with the router. A secondary pane renders the same screen but owns no route, so it
// must open things in itself: left on the default it would re-route pane 0 instead.
import { router } from "expo-router";
import { createContext, useContext } from "react";

import { openSession as openRoutedSession } from "../../lib/nav";
import type { WorkbenchSurfaceInput } from "../workbench/model";

export interface PaneNav {
  openSession(id: string): void;
  openAgents(sessionId: string): void;
}

export const defaultPaneNav: PaneNav = {
  openSession: openRoutedSession,
  openAgents: (sessionId) => router.push(`/session/${sessionId}/agents`),
};

export const PaneNavContext = createContext<PaneNav>(defaultPaneNav);

export function usePaneNav(): PaneNav {
  return useContext(PaneNavContext);
}

interface PaneNavWorkbench {
  openSession(id: string, mode: { pane: number }): void;
  focusPane(index: number): void;
  openSurface(input: WorkbenchSurfaceInput): void;
}

/** The nav for secondary pane `index`: sessions open in that pane, agents in the dock it focuses. */
export function secondaryPaneNav(workbench: PaneNavWorkbench, index: number): PaneNav {
  return {
    openSession: (id) => workbench.openSession(id, { pane: index }),
    openAgents: () => {
      workbench.focusPane(index);
      workbench.openSurface({ kind: "agents" });
    },
  };
}
