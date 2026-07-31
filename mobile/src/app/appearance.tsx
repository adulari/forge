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
import { setWrapCodeBlocks, useAppearancePreferences } from "../lib/appearancePreferences";
import { useTheme } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

export default function AppearanceScreen() {
  const { preference, setScheme, tokens } = useTheme();
  const appearance = useAppearancePreferences();
  const toast = useToast();

  const updateCodeWrap = (value: boolean) => {
    void setWrapCodeBlocks(value).catch(() => {
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
