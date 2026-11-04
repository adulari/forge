// The collapsible bottom panel. Terminal is the first tenant (the existing dock, loaded on first
// open); Checks and Shells join it as tabs in the next change.
import { Terminal as TerminalIcon, X } from "lucide-react-native";
import React, { Suspense, useCallback, useRef } from "react";
import { StyleSheet, Text, View } from "react-native";

import { IDE_BOTTOM_HEIGHT, persistIdeModePreferences, setIdeBottomHeight, useIdeModePreferences } from "../../lib/ideModePrefs";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { sansFamilyFor } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { ResizeHandle } from "../shell/ResizeHandle";
import { useIde } from "./IdeContext";
import { IdeIconButton } from "./ideUi";

const TerminalDock = React.lazy(() => import("../shell/TerminalDock").then((m) => ({ default: m.TerminalDock })));

export function BottomPanel() {
  const tokens = useTokens();
  const { bottomOpen, toggleBottom, panelSessionId } = useIde();
  const { bottomHeight } = useIdeModePreferences();
  const start = useRef(bottomHeight);
  const onStart = useCallback(() => {
    start.current = bottomHeight;
  }, [bottomHeight]);
  const onDrag = useCallback((delta: number) => setIdeBottomHeight(start.current - delta, false), []);
  if (!bottomOpen) return null;
  return (
    <View style={[styles.root, { height: bottomHeight, borderTopColor: tokens.border, backgroundColor: tokens.bg0 }]}>
      <ResizeHandle
        orientation="horizontal"
        overlay="start"
        label="Resize bottom panel"
        onStart={onStart}
        onDrag={onDrag}
        onEnd={persistIdeModePreferences}
        onReset={() => setIdeBottomHeight(IDE_BOTTOM_HEIGHT.default)}
        onNudge={(delta) => setIdeBottomHeight(bottomHeight - delta)}
      />
      <View style={[styles.header, { borderBottomColor: tokens.hairline }]}>
        <TerminalIcon size={12} strokeWidth={1.75} color={tokens.ink2} />
        <Text style={[styles.title, { color: tokens.ink }]}>Terminal</Text>
        <View style={styles.spacer} />
        <IdeIconButton icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />} label="Close terminal" onPress={toggleBottom} />
      </View>
      <View style={styles.body}>
        {panelSessionId ? (
          <Suspense fallback={<View style={styles.body} />}>
            <TerminalDock sessionId={panelSessionId} />
          </Suspense>
        ) : (
          <EmptyState icon={TerminalIcon} message="Pick an agent and its terminal opens here." />
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { borderTopWidth: StyleSheet.hairlineWidth },
  header: { height: 30, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  title: { fontFamily: sansFamilyFor(500), fontSize: 12 },
  spacer: { flex: 1 },
  body: { flex: 1, minHeight: 0 },
});
