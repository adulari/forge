// Left rail: project, the agents on it, then Changes / Files / Git for the agent in view.
import { ChevronDown } from "lucide-react-native";
import React, { useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { PopoverMenu, type MenuAnchor, type MenuEntry } from "../shell/PopoverMenu";
import { AgentsList } from "./AgentsList";
import { ChangeQueue } from "./ChangeQueue";
import { FilesTab } from "./FilesTab";
import { GitTab } from "./GitTab";
import { useIde, type RailTab } from "./IdeContext";
import { rowBackground } from "./rowStyle";

function ProjectSwitcher() {
  const tokens = useTokens();
  const { project, projects, setProject } = useIde();
  const ref = useRef<View>(null);
  const { width: windowWidth } = useWindowDimensions();
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const entries = useMemo<MenuEntry[]>(
    () =>
      projects.map((chip) => ({
        kind: "item",
        id: chip.name,
        label: chip.name,
        hint: chip.waiting > 0 ? `${chip.waiting} waiting` : String(chip.count),
        checked: chip.name === project,
        onPress: () => {
          setProject(chip.name);
          setAnchor(null);
        },
      })),
    [projects, project, setProject],
  );
  const open = () =>
    ref.current?.measureInWindow((x, y, _w, h) => setAnchor({ top: y + h + 2, left: Math.min(x, Math.max(8, windowWidth - 288)) }));
  const many = projects.length > 1;
  return (
    <>
      <Pressable
        ref={ref}
        onPress={many ? open : undefined}
        accessibilityRole="button"
        accessibilityLabel={`Project ${project ?? "none"}${many ? ". Change project" : ""}`}
        style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.project, { backgroundColor: rowBackground(tokens, { hovered: hovered && many }) }]}
      >
        <Text numberOfLines={1} style={[styles.projectName, { color: project ? tokens.ink : tokens.ink3 }]}>
          {project ?? "No project"}
        </Text>
        {many ? <ChevronDown size={12} strokeWidth={2} color={tokens.ink3} /> : null}
      </Pressable>
      <PopoverMenu anchor={anchor} entries={entries} onClose={() => setAnchor(null)} accessibilityLabel="Projects" width={260} />
    </>
  );
}

const TABS: readonly { value: RailTab; label: string }[] = [
  { value: "changes", label: "Changes" },
  { value: "files", label: "Files" },
  { value: "git", label: "Git" },
];

function RailTabs() {
  const tokens = useTokens();
  const { railTab, setRailTab, rows } = useIde();
  const toReview = rows.filter((row) => row.tier !== "reviewed").length;
  return (
    <View style={[styles.tabs, { borderTopColor: tokens.border, borderBottomColor: tokens.border }]} accessibilityRole="tablist">
      {TABS.map((tab) => {
        const selected = tab.value === railTab;
        return (
          <Pressable
            key={tab.value}
            onPress={() => setRailTab(tab.value)}
            accessibilityRole="tab"
            accessibilityState={{ selected }}
            style={styles.tab}
          >
            <Text style={[styles.tabText, { color: selected ? tokens.ink : tokens.ink3 }]}>{tab.label}</Text>
            {tab.value === "changes" && toReview > 0 ? <Text style={[styles.tabCount, { color: tokens.ink3 }]}>{toReview}</Text> : null}
            {selected ? <View style={[styles.underline, { backgroundColor: tokens.ink }]} /> : null}
          </Pressable>
        );
      })}
    </View>
  );
}

export function Rail() {
  const tokens = useTokens();
  const { railTab } = useIde();
  return (
    <View style={[styles.root, { backgroundColor: tokens.bg1 }]}>
      <ProjectSwitcher />
      <AgentsList />
      <RailTabs />
      <View style={styles.body}>
        {railTab === "changes" ? <ChangeQueue /> : railTab === "files" ? <FilesTab /> : <GitTab />}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minHeight: 0 },
  project: { height: 36, flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingHorizontal: space.space12 },
  projectName: { flexShrink: 1, fontFamily: sansFamilyFor(600), fontSize: 13, letterSpacing: -0.1 },
  tabs: { height: 32, flexDirection: "row", alignItems: "stretch", paddingHorizontal: space.space8, borderTopWidth: StyleSheet.hairlineWidth, borderBottomWidth: StyleSheet.hairlineWidth, marginTop: space.space4 },
  tab: { flexDirection: "row", alignItems: "center", gap: 5, paddingHorizontal: space.space8 },
  tabText: { fontFamily: sansFamilyFor(500), fontSize: 12 },
  tabCount: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  underline: { position: "absolute", left: space.space8, right: space.space8, bottom: -StyleSheet.hairlineWidth, height: 1.5 },
  body: { flex: 1, minHeight: 0 },
});
