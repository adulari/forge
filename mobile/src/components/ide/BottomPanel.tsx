// The collapsible bottom panel: Terminal (the existing dock), Checks and Shells. Each tab is its own
// chunk, loaded the first time it is shown.
import { ListChecks, Terminal as TerminalIcon, X } from "lucide-react-native";
import React, { Suspense, useCallback, useRef } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { IDE_BOTTOM_HEIGHT, persistIdeModePreferences, setIdeBottomHeight, useIdeModePreferences } from "../../lib/ideModePrefs";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { sansFamilyFor } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { ResizeHandle } from "../shell/ResizeHandle";
import { useIde } from "./IdeContext";
import { IDE_DEFAULT_TERMINAL, failingPathsOf, setBottomTab, setIdeTerminalId, useBottomTab, useFailingChecks, useIdeTerminalId, type BottomTab } from "./ideSignals";
import { IdeIconButton } from "./ideUi";

const TerminalDock = React.lazy(() => import("../shell/TerminalDock").then((m) => ({ default: m.TerminalDock })));

const ChecksPanel = React.lazy(() => import("./ChecksPanel"));
const ShellsPanel = React.lazy(() => import("./ShellsPanel"));

const TABS: { id: BottomTab; label: string }[] = [
  { id: "terminal", label: "Terminal" },
  { id: "checks", label: "Checks" },
  { id: "shells", label: "Shells" },
];

export function BottomPanel() {
  const tokens = useTokens();
  const { bottomOpen, toggleBottom, panelSessionId } = useIde();
  const { bottomHeight } = useIdeModePreferences();
  const tab = useBottomTab();
  const terminalId = useIdeTerminalId();
  const failing = useFailingChecks();
  const failingCount = panelSessionId ? failingPathsOf(failing, panelSessionId).size : 0;
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
      <View style={[styles.header, { borderBottomColor: tokens.hairline }]} accessibilityRole="tablist">
        {TABS.map((item) => (
          <Pressable
            key={item.id}
            onPress={() => setBottomTab(item.id)}
            accessibilityRole="tab"
            accessibilityState={{ selected: tab === item.id }}
            style={[styles.tab, tab === item.id && { borderBottomColor: tokens.ink }]}
          >
            {item.id === "terminal" ? <TerminalIcon size={12} strokeWidth={1.75} color={tab === item.id ? tokens.ink : tokens.ink3} /> : null}
            {item.id === "checks" ? <ListChecks size={12} strokeWidth={1.75} color={tab === item.id ? tokens.ink : tokens.ink3} /> : null}
            <Text style={[styles.title, { color: tab === item.id ? tokens.ink : tokens.ink3 }]}>{item.label}</Text>
            {item.id === "checks" && failingCount > 0 ? <Text style={[styles.badge, { color: tokens.danger }]}>{`${failingCount} failing`}</Text> : null}
          </Pressable>
        ))}
        {terminalId !== IDE_DEFAULT_TERMINAL && tab === "terminal" ? (
          <Pressable onPress={() => setIdeTerminalId(IDE_DEFAULT_TERMINAL)} accessibilityRole="button" accessibilityLabel="Back to the shell terminal" style={styles.tab}>
            <Text style={[styles.title, { color: tokens.ink3 }]}>Back to shell</Text>
          </Pressable>
        ) : null}
        <View style={styles.spacer} />
        <IdeIconButton icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />} label="Close panel" onPress={toggleBottom} />
      </View>
      <View style={styles.body}>
        {!panelSessionId ? (
          <EmptyState icon={TerminalIcon} message="Pick an agent and its terminal, checks and shells open here." />
        ) : (
          <Suspense fallback={<View style={styles.body} />}>
            {tab === "terminal" ? <TerminalDock key={terminalId} sessionId={panelSessionId} terminalId={terminalId} /> : null}
            {tab === "checks" ? <ChecksPanel key={panelSessionId} sessionId={panelSessionId} /> : null}
            {tab === "shells" ? <ShellsPanel key={panelSessionId} sessionId={panelSessionId} /> : null}
          </Suspense>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { borderTopWidth: StyleSheet.hairlineWidth },
  header: { height: 30, flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  title: { fontFamily: sansFamilyFor(500), fontSize: 12 },
  tab: { height: 30, flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: space.space8, borderBottomWidth: 1.5, borderBottomColor: "transparent" },
  badge: { fontFamily: sansFamilyFor(600), fontSize: 10.5 },
  spacer: { flex: 1 },
  body: { flex: 1, minHeight: 0 },
});
