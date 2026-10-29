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
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import * as Updates from "expo-updates";

import { useSettledAppVersion } from "../lib/appVersion";
import { useAuth } from "../lib/auth";
import { releaseForApp, releaseNotesUrl, splitEntry } from "../lib/changelogEntry";
import { useChangelog } from "../lib/queries";
import { isTauri } from "../lib/platform";
import {
  loadLastSeenBuild,
  rememberBuild,
  updateNotice,
  type UpdateNotice as Notice,
} from "../lib/updateNotice";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { type } from "../theme/typography";
import { Markdown } from "./chat/Markdown";
import { ReleaseHero } from "./shell/ReleaseHero";
import { Button } from "./ds/Button";
import { Sheet } from "./ds/Sheet";

export function UpdateNotice() {
  const tokens = useTokens();
  const appVersion = useSettledAppVersion();
  const { isPaired } = useAuth();
  const [notice, setNotice] = React.useState<Notice | null>(null);
  const changelog = useChangelog(isTauri ? 20 : 1);

  React.useEffect(() => {
    if (appVersion == null) return;
    let cancelled = false;
    void (async () => {
      let seedUpdateSeen = false;
      if (isTauri) {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          seedUpdateSeen = await invoke<boolean>("perf_seed_update_seen");
        } catch {
          seedUpdateSeen = false;
        }
      }
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

  const release = releaseForApp(changelog.data, notice.appVersion, isTauri);
  const heading = notice.kind === "app"
    ? `What's new in v${notice.appVersion}`
    : "Installed Forge update";
  const subtitle =
    notice.kind === "app"
      ? "This is the version now running on this device."
      : "The app updated itself in the background; this build is now running.";

  return (
    <Sheet visible onClose={() => setNotice(null)} accessibilityLabel="What's new">
      <View style={styles.body}>
        <Text style={[type.headingBold, { color: tokens.ink }]}>{heading}</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>{subtitle}</Text>

        {release ? (
          <>
            <ReleaseHero release={release} />
            <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>
              {release.version}
              {release.date ? ` · ${release.date}` : ""}
            </Text>
            <ScrollView style={styles.entries} contentContainerStyle={styles.entryList}>
              {release.entries.map((entry, position) => (
                <Entry key={`${entry.section}-${position}`} text={entry.text} />
              ))}
            </ScrollView>
          </>
        ) : (
          <Text style={[type.meta, styles.section, { color: tokens.ink4 }]}>
            {/* The notes live on the daemon. Without one paired, or with an older one that has no
                section for this version, say so instead of showing another release's notes. */}
            {changelog.data?.length
              ? "Your server runs an older Forge, so it has no notes for this version yet."
              : "Connect a server to read what changed."}
          </Text>
        )}

        {isTauri && notice.kind === "app" ? (
          <Pressable
            onPress={() => void openReleaseNotes(notice.appVersion)}
            accessibilityRole="link"
            style={styles.link}
          >
            <Text style={[type.sub, { color: tokens.accent }]}>Full release notes</Text>
          </Pressable>
        ) : null}

        <Button label="Got it" onPress={() => setNotice(null)} />
      </View>
    </Sheet>
  );
}

async function openReleaseNotes(version: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(releaseNotesUrl(version)).catch(() => undefined);
}

/** The bold summary always shows; the detail opens on tap so the sheet stays short. */
function Entry({ text }: { text: string }) {
  const tokens = useTokens();
  const [open, setOpen] = React.useState(false);
  const { lead, detail } = splitEntry(text);
  return (
    <Pressable
      onPress={() => setOpen((value) => !value)}
      disabled={!detail}
      accessibilityRole={detail ? "button" : undefined}
      accessibilityState={detail ? { expanded: open } : undefined}
      style={styles.entry}
    >
      <View style={[styles.bullet, { backgroundColor: tokens.accent }]} />
      <View style={styles.entryText}>
        <Markdown content={lead} style={[type.sub, { color: tokens.ink }]} />
        {detail && open ? <Markdown content={detail} style={[type.sub, { color: tokens.ink3 }]} /> : null}
        {detail && !open ? <Text style={[type.meta, { color: tokens.ink4 }]}>Details</Text> : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  body: { gap: space.space8, paddingHorizontal: space.space16, paddingBottom: space.space16 },
  section: { marginTop: space.space8 },
  entries: { maxHeight: 320, marginBottom: space.space8 },
  entryList: { gap: space.space8, paddingVertical: space.space8 },
  entry: { flexDirection: "row", gap: space.space8, alignItems: "flex-start" },
  bullet: { width: 5, height: 5, borderRadius: radii.radius4, marginTop: 7 },
  entryText: { flex: 1, gap: 2 },
  link: { alignSelf: "flex-start", paddingVertical: space.space4 },
});
