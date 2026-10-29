import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { ThemePicker } from "../components/settings/ThemePicker";
import { TerminalSettingsSection } from "../components/settings/TerminalSettingsSection";
import { setFoldToolGroups, setWrapCodeBlocks, useAppearancePreferences } from "../lib/appearancePreferences";
import { setDockPlacement, useDockPrefs } from "../lib/composerDocks";
import { useTheme } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

export default function AppearanceScreen() {
  const { preference, setScheme, tokens } = useTheme();
  const appearance = useAppearancePreferences();
  const toast = useToast();
  const docks = useDockPrefs();

  const updateCodeWrap = (value: boolean) => {
    void setWrapCodeBlocks(value).catch(() => {
      toast.show("couldn't save appearance preference.", { tone: "danger" });
    });
  };

  const updateFoldToolGroups = (value: boolean) => {
    void setFoldToolGroups(value).catch(() => {
      toast.show("couldn't save appearance preference.", { tone: "danger" });
    });
  };

  return (
    <DesktopDrillDown>
      <SettingsShell active="appearance">
        <Screen scroll contentContainerStyle={styles.content}>
          <View style={styles.header}>
            <BackLink />
            <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>Appearance</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              Display preferences apply immediately and stay on this device.
            </Text>
          </View>

          <View>
            <SectionHeader>Theme</SectionHeader>
            <Segmented
              options={[
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
                { value: "system", label: "System" },
              ]}
              value={preference}
              onChange={setScheme}
            />
            <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
              System follows the operating system without storing a resolved light or dark value.
            </Text>
          </View>

          <ThemePicker />

          <View>
            <SectionHeader>Reading code</SectionHeader>
            <ListRow
              title="Wrap code blocks"
              subtitle="Wrap long assistant code to the available width instead of scrolling horizontally."
              showSeparator={false}
              trailing={
                <Switch
                  value={appearance.wrapCodeBlocks}
                  onValueChange={updateCodeWrap}
                  disabled={!appearance.loaded}
                  accessibilityLabel="Wrap code blocks"
                />
              }
            />
            <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
              Git diffs remain horizontally scrollable so side-by-side alignment and line annotations stay exact.
            </Text>
          </View>

          <View>
            <SectionHeader>Transcript</SectionHeader>
            <ListRow
              title="Fold tool calls"
              subtitle="Show each run of tool calls as one summary row, such as “Read 3 files, ran 2 shell commands”. Tap a row to see every call."
              showSeparator={false}
              trailing={
                <Switch
                  value={appearance.foldToolGroups}
                  onValueChange={updateFoldToolGroups}
                  disabled={!appearance.loaded}
                  accessibilityLabel="Fold tool calls"
                />
              }
            />
            <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
              Failed calls and calls still running stay listed under their summary.
            </Text>
          </View>

          <TerminalSettingsSection />

          <View>
            <SectionHeader>Thread docks</SectionHeader>
            <Segmented
              options={[
                { value: "composer", label: "Above the composer" },
                { value: "panel", label: "Side panel" },
              ]}
              value={docks.placement}
              onChange={(value) => void setDockPlacement(value)}
            />
            <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
              Where the task plan lives while a chat works. The side panel needs a wide window and opens from Layout or
              with the Tasks shortcut; narrower windows always keep it above the composer. The spend dock stays above
              the composer, and every dock can be moved or hidden from its menu.
            </Text>
          </View>
        </Screen>
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: {
    paddingTop: space.space12,
    paddingBottom: space.space48,
    gap: space.space24,
  },
  header: {
    gap: space.space8,
  },
  note: {
    paddingHorizontal: space.space16,
    paddingTop: space.space8,
  },
});
