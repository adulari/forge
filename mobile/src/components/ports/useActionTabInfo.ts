import { useEffect, useMemo } from "react";

import { useProjectActions } from "../../lib/queriesProjectActions";
import type { WorkbenchSurface } from "../workbench/model";
import { rememberActionNames } from "./actionTerminalNames";
import { ACTION_TERMINAL_PREFIX } from "./portsModel";

export interface ActionTabInfo {
  name: string;
  running: boolean;
}

export function hasActionTerminal(tabs: readonly WorkbenchSurface[]): boolean {
  return tabs.some((tab) => tab.kind === "terminal" && (tab.resourceId ?? "").startsWith(ACTION_TERMINAL_PREFIX));
}

/** Action name and running state per `act-` terminal id, fetched only while such a tab exists. */
export function useActionTabInfo(sessionId: string | null, enabled: boolean): ReadonlyMap<string, ActionTabInfo> {
  const { data } = useProjectActions(sessionId, enabled);
  useEffect(() => {
    if (data) rememberActionNames(data.actions);
  }, [data]);
  return useMemo(() => {
    const info = new Map<string, ActionTabInfo>();
    if (!enabled || !data) return info;
    for (const action of data.actions) info.set(`${ACTION_TERMINAL_PREFIX}${action.id}`, { name: action.name, running: action.running });
    return info;
  }, [data, enabled]);
}
