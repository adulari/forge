import { Play, Square } from "lucide-react-native";
import React, { useEffect, useMemo } from "react";

import { useProjectActions } from "../../lib/queriesProjectActions";
import { useTokens } from "../../theme/ThemeProvider";
import type { PaletteItem } from "../overlay/paletteTypes";
import { rememberActionNames } from "./actionTerminalNames";
import { actionPaletteEntries } from "./portsModel";
import { useActionControls } from "./useActionControls";

/** "Run: x" / "Stop: x" palette rows for the focused session. Queried only while the palette is open. */
export function useActionPaletteItems(sessionId: string | null, visible: boolean, close: () => void): PaletteItem[] {
  const tokens = useTokens();
  const actions = useProjectActions(sessionId, visible, false);
  const controls = useActionControls();
  const data = actions.data;

  useEffect(() => {
    if (data) rememberActionNames(data.actions);
  }, [data]);

  return useMemo(() => {
    if (!visible || !sessionId || !data) return [];
    return actionPaletteEntries(data.actions).map((entry): PaletteItem => {
      const action = data.actions.find((candidate) => candidate.id === entry.actionId);
      const Icon = entry.kind === "run" ? Play : Square;
      return {
        id: `action:${entry.id}`,
        section: "commands",
        title: entry.title,
        subtitle: action?.command,
        keywords: entry.keywords,
        leading: <Icon size={18} strokeWidth={1.75} color={tokens.ink2} />,
        onSelect: () => {
          close();
          if (!action) return;
          if (entry.kind === "run") controls.run(sessionId, action);
          else controls.stop(sessionId, action);
        },
      };
    });
  }, [close, controls, data, sessionId, tokens, visible]);
}
