// Tells the user when the app has changed under them, and what changed.
//
// An OTA is applied silently on the launch after it downloads, and a native build arrives through
// TestFlight with nothing in-app to mark it — so "did it update?" was unanswerable without reading
// CI. This closes that: one sheet, once per update, with the newest changelog section in it.
//
// The changelog comes from the daemon (`GET /api/changelog`, compiled into the binary), so it
// describes the DAEMON's release rather than the phone's bundle. That is the right thing to show —
// it is where the features are described — but it means the sheet stays empty rather than lying when
// no server is paired.
import React from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import * as Updates from "expo-updates";

import { useAppVersion } from "../lib/appVersion";
import { useAuth } from "../lib/auth";
import { useChangelog } from "../lib/queries";
import {
  loadLastSeenBuild,
  rememberBuild,
  updateNotice,
  type UpdateNotice as Notice,
} from "../lib/updateNotice";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { type } from "../theme/typography";
import { Button } from "./ds/Button";
import { Sheet } from "./ds/Sheet";

export function UpdateNotice() {
  const tokens = useTokens();
  const appVersion = useAppVersion();
  const { isPaired } = useAuth();
  const [notice, setNotice] = React.useState<Notice | null>(null);
  const changelog = useChangelog(1);

  React.useEffect(() => {
    let cancelled = false;
    void (async () => {
      const { invoke } = await import("@tauri-apps/api/core");
      const seedUpdateSeen = await invoke<boolean>("perf_seed_update_seen");
      if (seedUpdateSeen) {
        await rememberBuild({ updateId: Updates.updateId ?? null, appVersion });
        return;
      }
      // The body of this sheet is the daemon's changelog, so before a server is connected it can
      // only say "connect a server to read what changed" — while covering the connect flow that
      // would let it. Defer WITHOUT recording the build as seen, so the notice still arrives the
      // first time the app is actually usable instead of being silently consumed on a launch that
      // could not display it.
      if (!isPaired) return;
      const seen = await loadLastSeenBuild();
      if (cancelled) return;
      const found = updateNotice({
        // `updateId` is null in dev and on a build running its embedded bundle.
        updateId: Updates.updateId ?? null,
        appVersion,
        lastSeenUpdateId: seen.updateId,
        lastSeenAppVersion: seen.appVersion,
      });
      // Recorded immediately, not on dismiss: a sheet the user swipes away without tapping is still
      // a sheet they have seen, and showing it again on the next launch would be worse than never
      // showing it at all.
      await rememberBuild({ updateId: Updates.updateId ?? null, appVersion });
      if (!cancelled && found) setNotice(found);
    })();
    return () => {
      cancelled = true;
    };
  }, [appVersion, isPaired]);

  if (!notice) return null;

  const release = changelog.data?.[0];
  const heading = notice.kind === "app" ? `Forge ${notice.appVersion}` : "Forge updated";
  const subtitle =
    notice.kind === "app"
      ? "A new build is installed."
      : "The app updated itself in the background.";

  return (
    <Sheet visible onClose={() => setNotice(null)} accessibilityLabel="What's new">
      <View style={styles.body}>
        <Text style={[type.headingBold, { color: tokens.ink }]}>{heading}</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>{subtitle}</Text>

        {release ? (
          <>
            <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>
              {release.version}
              {release.date ? ` · ${release.date}` : ""}
            </Text>
            <ScrollView style={styles.entries} contentContainerStyle={styles.entryList}>
              {release.entries.map((entry, position) => (
                <View key={`${entry.section}-${position}`} style={styles.entry}>
                  <View style={[styles.bullet, { backgroundColor: tokens.accent }]} />
                  <Text style={[type.sub, styles.entryText, { color: tokens.ink2 }]}>{entry.text}</Text>
                </View>
              ))}
            </ScrollView>
          </>
        ) : (
          <Text style={[type.meta, styles.section, { color: tokens.ink4 }]}>
            {/* Honest about WHY rather than showing an empty panel: the changelog lives on the
                daemon, so without one paired there is nothing to read. */}
            Connect a server to read what changed.
          </Text>
        )}

        <Button label="Got it" onPress={() => setNotice(null)} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: { gap: space.space8, paddingBottom: space.space8 },
  section: { marginTop: space.space8 },
  entries: { maxHeight: 320, marginBottom: space.space8 },
  entryList: { gap: space.space8, paddingVertical: space.space8 },
  entry: { flexDirection: "row", gap: space.space8, alignItems: "flex-start" },
  bullet: { width: 5, height: 5, borderRadius: radii.radius4, marginTop: 7 },
  entryText: { flex: 1, lineHeight: 19 },
});
