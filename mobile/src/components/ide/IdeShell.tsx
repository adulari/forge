// IDE mode: an agent review workbench in three panes. Rail (agents + changes/files/git), the
// editor (tabs, diff, file), and the agent panel, with a collapsible bottom panel and a hints bar.
// Rendered beside the chat shell inside <Activity>, so switching modes keeps both alive.
import React, { useCallback, useRef } from "react";
import { StyleSheet, View, useWindowDimensions } from "react-native";

import {
  IDE_AGENT_WIDTH,
  IDE_RAIL_WIDTH,
  persistIdeModePreferences,
  setIdeAgentWidth,
  setIdeRailWidth,
  useIdeModePreferences,
} from "../../lib/ideModePrefs";
import { useTokens } from "../../theme/ThemeProvider";
import { ResizeHandle } from "../shell/ResizeHandle";
import { AgentPanel } from "./AgentPanel";
import { BottomPanel } from "./BottomPanel";
import { CoachMarks } from "./CoachMarks";
import { EditorArea } from "./EditorArea";
import { IdeProvider } from "./IdeContext";
import { KeyHints } from "./KeyHints";
import { Rail } from "./Rail";
import { fitPanes } from "./paneFit";
import { useIdeShortcuts } from "./useIdeShortcuts";

export function IdeShell() {
  return (
    <IdeProvider>
      <Workbench />
    </IdeProvider>
  );
}

function Workbench() {
  const tokens = useTokens();
  useIdeShortcuts();
  const { width } = useWindowDimensions();
  const prefs = useIdeModePreferences();
  const fit = fitPanes(width, prefs.railWidth, prefs.agentWidth);

  const grab = useRef(0);
  const onRailStart = useCallback(() => {
    grab.current = fit.rail;
  }, [fit.rail]);
  const onAgentStart = useCallback(() => {
    grab.current = fit.agent;
  }, [fit.agent]);

  return (
    <View style={[styles.root, { backgroundColor: tokens.bg0 }]}>
      <View style={styles.main}>
        <View style={[styles.rail, { width: fit.rail, borderRightColor: tokens.border }]}>
          <Rail />
          <ResizeHandle
            orientation="vertical"
            overlay="end"
            label="Resize agents and changes"
            onStart={onRailStart}
            onDrag={(delta) => setIdeRailWidth(grab.current + delta, false)}
            onEnd={persistIdeModePreferences}
            onReset={() => setIdeRailWidth(IDE_RAIL_WIDTH.default)}
            onNudge={(delta) => setIdeRailWidth(fit.rail + delta)}
          />
        </View>
        <View style={styles.center}>
          <EditorArea />
          <BottomPanel />
        </View>
        <View style={[styles.agent, { width: fit.agent, borderLeftColor: tokens.border }]}>
          <AgentPanel />
          <ResizeHandle
            orientation="vertical"
            overlay="start"
            label="Resize agent panel"
            onStart={onAgentStart}
            onDrag={(delta) => setIdeAgentWidth(grab.current - delta, false)}
            onEnd={persistIdeModePreferences}
            onReset={() => setIdeAgentWidth(IDE_AGENT_WIDTH.default)}
            onNudge={(delta) => setIdeAgentWidth(fit.agent - delta)}
          />
        </View>
      </View>
      <KeyHints />
      <CoachMarks railWidth={fit.rail} agentWidth={fit.agent} />
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minHeight: 0 },
  main: { flex: 1, flexDirection: "row", minHeight: 0 },
  rail: { borderRightWidth: StyleSheet.hairlineWidth },
  center: { flex: 1, minWidth: 0, minHeight: 0 },
  agent: { borderLeftWidth: StyleSheet.hairlineWidth },
});
