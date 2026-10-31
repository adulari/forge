import { useEffect, useMemo } from "react";

import { useProjectActions } from "../../lib/queriesProjectActions";
import type { MenuEntry } from "../shell/PopoverMenu";
import { rememberActionNames } from "./actionTerminalNames";
import { useActionControls } from "./useActionControls";

function note(id: string, label: string, hint?: string): MenuEntry {
  return { kind: "item", id, label, hint, disabled: true, onPress: () => undefined };
}

/**
 * The "Run" section of a session or project menu. Actions are fetched only while the menu is
 * open, so a long sidebar does not poll once per row.
 */
export function useActionMenuEntries(sessionId: string | null, open: boolean): MenuEntry[] {
  const actions = useProjectActions(sessionId, open, false);
  const controls = useActionControls();
  const data = actions.data;

  useEffect(() => {
    if (data) rememberActionNames(data.actions);
  }, [data]);

  return useMemo(() => {
    if (!open || sessionId == null) return [];
    const head: MenuEntry[] = [{ kind: "separator", id: "sep-run" }, { kind: "heading", id: "h-run", label: "RUN" }];
    if (!controls.available) return [...head, note("run-na", "Needs a direct connection")];
    if (actions.isPending) return [...head, note("run-loading", "Loading actions")];
    if (actions.isError || !data) return [...head, note("run-error", "Could not load actions")];
    if (data.untrusted) return [...head, note("run-untrusted", "Project not trusted", "run forge trust")];
    if (data.actions.length === 0) return [...head, note("run-none", "No actions configured", "[[actions]]")];
    return [
      ...head,
      ...data.actions.map(
        (action): MenuEntry => ({
          kind: "item",
          id: `run-${action.id}`,
          label: action.running ? `Stop ${action.name}` : `Run ${action.name}`,
          hint: action.running ? "running" : undefined,
          onPress: () => {
            if (action.running) controls.stop(sessionId, action);
            else controls.run(sessionId, action);
          },
        }),
      ),
    ];
  }, [actions.isError, actions.isPending, controls, data, open, sessionId]);
}
