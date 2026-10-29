import { router } from "expo-router";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { getLaunchSettings, setLaunchAtLogin, setStartMinimized, type LaunchSettings } from "../lib/desktopLaunch";
import {
  setNotifyWhenBackground,
  setUsageRefreshMinutes,
  useDesktopShellPrefs,
} from "../lib/desktopShellPrefs";
import { isTauri } from "../lib/platform";
import { formatShortcut, useShortcutBinding } from "../lib/shortcuts";
import { useGlobalChordStatus } from "../lib/shortcuts/globalChord";
import { describeChordStatus } from "../lib/shortcuts/globalChordCore";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const REFRESH_CHOICES = [2, 5, 10, 15, 30] as const;

export default function DesktopScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const prefs = useDesktopShellPrefs();
  const chord = useShortcutBinding("shell.quickComposer");
  const chordNote = describeChordStatus(useGlobalChordStatus());
  const [launch, setLaunch] = useState<LaunchSettings | null>(null);

  useEffect(() => {
    void getLaunchSettings().then(setLaunch);
  }, []);

  const failed = useCallback(
    (what: string) => toast.show(`couldn't save ${what}.`, { tone: "danger" }),
    [toast],
  );

  const changeLaunchAtLogin = (value: boolean) => {
    const previous = launch;
    setLaunch((current) => (current ? { ...current, launchAtLogin: value } : current));
    setLaunchAtLogin(value).catch(() => {
      setLaunch(previous);
      failed("the launch setting");
    });
  };

  const changeStartMinimized = (value: boolean) => {
    const previous = launch;
    setLaunch((current) => (current ? { ...current, startMinimized: value } : current));
    setStartMinimized(value).catch(() => {
      setLaunch(previous);
      failed("the launch setting");
    });
  };

  const refreshValue = String(
    REFRESH_CHOICES.reduce((best, choice) =>
      Math.abs(choice - prefs.usageRefreshMinutes) < Math.abs(best - prefs.usageRefreshMinutes) ? choice : best,
    ),
  );

  return (
    <DesktopDrillDown>
      <SettingsShell active="desktop">
        <Screen scroll contentContainerStyle={styles.content}>
          <View style={styles.header}>
            <BackLink />
            <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>Desktop</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              How Forge behaves as an app on this computer. These stay on this device.
            </Text>
          </View>

          {!isTauri ? (
            <Text style={[type.sub, { color: tokens.ink3 }]}>These settings belong to the Forge desktop app.</Text>
          ) : (
            <>
              <View>
                <SectionHeader>Startup</SectionHeader>
                <ListRow
                  title="Launch at login"
                  subtitle="Open Forge when you sign in to your computer."
                  trailing={
                    <Switch
                      value={launch?.launchAtLogin ?? false}
                      onValueChange={changeLaunchAtLogin}
                      disabled={launch == null}
                      accessibilityLabel="Launch at login"
                    />
                  }
                />
                <ListRow
                  title="Start minimized"
                  subtitle="When launched at login, stay in the tray instead of opening the window."
                  showSeparator={false}
                  trailing={
                    <Switch
                      value={launch?.startMinimized ?? false}
                      onValueChange={changeStartMinimized}
                      disabled={launch == null || !launch.launchAtLogin}
                      accessibilityLabel="Start minimized"
                    />
                  }
                />
              </View>

              <View>
                <SectionHeader>Quick composer</SectionHeader>
                <ListRow
                  title="Shortcut"
                  subtitle={chordNote?.text ?? "Opens the quick composer from any app."}
                  onPress={() => router.push("/keybindings")}
                  showSeparator={false}
                  trailing={<Text style={[type.monoMeta, { color: chordNote?.tone === "warn" ? tokens.warn : tokens.ink2 }]}>{formatShortcut(chord)}</Text>}
                />
              </View>

              <View>
                <SectionHeader>Notifications</SectionHeader>
                <ListRow
                  title="Notify when Forge is in the background"
                  subtitle="A system notification when a session needs you while this window isn't focused."
                  showSeparator={false}
                  trailing={
                    <Switch
                      value={prefs.notifyWhenBackground}
                      onValueChange={(value) => void setNotifyWhenBackground(value).catch(() => failed("the notification setting"))}
                      disabled={!prefs.loaded}
                      accessibilityLabel="Notify when Forge is in the background"
                    />
                  }
                />
              </View>

              <View>
                <SectionHeader>Usage</SectionHeader>
                <View style={styles.segmentRow}>
                  <Text style={[type.body, { color: tokens.ink }]}>Refresh usage every</Text>
                  <Segmented
                    options={REFRESH_CHOICES.map((minutes) => ({ value: String(minutes), label: `${minutes} min` }))}
                    value={refreshValue}
                    onChange={(value) => void setUsageRefreshMinutes(Number(value)).catch(() => failed("the refresh interval"))}
                  />
                </View>
                <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
                  Applies to the account-wide usage figures. A running session&apos;s own usage keeps updating live.
                </Text>
              </View>
            </>
          )}
        </Screen>
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48, gap: space.space24 },
  header: { gap: space.space8 },
  segmentRow: { paddingHorizontal: space.space16, gap: space.space8 },
  note: { paddingHorizontal: space.space16, paddingTop: space.space8 },
});
