import { useCallback } from "react";
import { Linking, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { useAuth } from "../lib/auth";
import { goBackOr } from "../lib/nav";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

const LINKS = {
  privacy: "https://github.com/adulari/forge/blob/main/docs/mobile/PRIVACY.md",
  license: "https://github.com/adulari/forge/blob/main/LICENSE",
  support: "https://github.com/adulari/forge/issues",
  source: "https://github.com/adulari/forge",
} as const;

export default function LegalScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { isPaired } = useAuth();
  const open = useCallback((url: string) => {
    void Linking.openURL(url).catch(() => {
      toast.show("Couldn't open that link.", { tone: "danger" });
    });
  }, [toast]);

  const screen = (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <BackLink
          label={isPaired ? "Settings" : "Connect"}
          onPress={() => goBackOr(isPaired ? "/settings" : "/connect")}
        />
        <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>
          Legal & support
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          Forge is open source. These links open in your browser and are available before
          contacting support or submitting diagnostics.
        </Text>
      </View>
      <View>
        <SectionHeader>Documents</SectionHeader>
        <ListRow
          title="Privacy policy"
          subtitle="What Forge stores, sends, and deliberately never collects"
          onPress={() => open(LINKS.privacy)}
        />
        <ListRow
          title="Open-source license"
          subtitle="GNU Affero General Public License v3.0"
          onPress={() => open(LINKS.license)}
          showSeparator={false}
        />
      </View>
      <View>
        <SectionHeader>Help & source</SectionHeader>
        <ListRow
          title="Report an issue"
          subtitle="Public support and bug reports on GitHub"
          onPress={() => open(LINKS.support)}
        />
        <ListRow
          title="Forge source code"
          subtitle="Builds, releases, documentation, and contribution guide"
          onPress={() => open(LINKS.source)}
          showSeparator={false}
        />
      </View>
    </Screen>
  );

  return (
    <DesktopDrillDown>
      {isPaired ? <SettingsShell active="legal">{screen}</SettingsShell> : screen}
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: {
    width: "100%",
    maxWidth: 720,
    alignSelf: "center",
    gap: space.space24,
    paddingHorizontal: space.space16,
    paddingTop: space.space24,
    paddingBottom: space.space48,
  },
  header: { gap: space.space8 },
});
