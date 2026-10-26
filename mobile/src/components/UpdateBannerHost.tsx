import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Platform } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { useApplyDaemonUpdate, useDaemonUpdate, useIdentityUpdateHint } from "../lib/queries";
import { isTauri } from "../lib/platform";
import {
  bannerActionLabel,
  bannerMessage,
  canSkip,
  chooseUpdateBanner,
  loadPrefs,
  savePrefs,
  skip,
  snooze,
  type UpdatePrefs,
} from "../lib/updateBanner";
import { installDesktopUpdate, openDesktopReleasePage, useDesktopUpdateState } from "../lib/updater";
import { Banner } from "./ds/Banner";
import { useToast } from "./ds/ToastHost";

const INSTALLED_EVENT = "forge://desktop-update-installed";

/** Version a background self-install left on disk, from the native event or the late-mount query. */
function useInstalledVersion(): string | null {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    let active = true;
    void (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const { listen } = await import("@tauri-apps/api/event");
        const already = await invoke<string | null>("desktop_update_installed_version");
        if (active && already) setVersion(already);
        const off = await listen<string>(INSTALLED_EVENT, (event) => setVersion(event.payload));
        if (active) unlisten = off;
        else off();
      } catch {
        // Not running under the Tauri shell this build expects: no restart banner.
      }
    })();
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
  return version;
}

/** The one update banner: restart, app update, or CLI/daemon update, with real actions. */
export function UpdateBannerHost({ appVersion }: { appVersion: string }) {
  const toast = useToast();
  const desktop = useDesktopUpdateState();
  const daemonQuery = useDaemonUpdate(isTauri);
  const identityHint = useIdentityUpdateHint(!isTauri);
  const applyDaemon = useApplyDaemonUpdate();
  const installedVersion = useInstalledVersion();
  const [prefs, setPrefs] = useState<UpdatePrefs>(loadPrefs);
  const [busy, setBusy] = useState(false);
  const [now] = useState(() => Date.now());

  const banner = useMemo(
    () =>
      chooseUpdateBanner({
        isDesktop: isTauri,
        desktop,
        installedVersion,
        daemon: daemonQuery.data ?? null,
        identityLatest: identityHint.data ?? null,
        prefs,
        now,
      }),
    [daemonQuery.data, desktop, identityHint.data, installedVersion, now, prefs],
  );

  const update = useCallback((next: UpdatePrefs) => {
    setPrefs(next);
    savePrefs(next);
  }, []);

  const act = useCallback(async () => {
    if (!banner || busy) return;
    setBusy(true);
    try {
      if (banner.kind === "app-restart") {
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await relaunch();
      } else if (banner.kind === "app-install") {
        await installDesktopUpdate();
      } else if (banner.kind === "app-download") {
        await openDesktopReleasePage(banner.url);
      } else if (banner.kind === "daemon-update") {
        const result = await applyDaemon.mutateAsync();
        const note = {
          scheduled: "Forge (CLI) updated. The service restarts now.",
          waiting_for_idle: "Forge (CLI) updated. The service restarts when no session is busy.",
          manual: "Forge (CLI) updated. Restart `forge serve` to use it.",
          not_needed: "Forge (CLI) is already up to date.",
        }[result.restart];
        toast.show(note, { tone: "neutral" });
        update(snooze(prefs, banner.key, Date.now()));
      }
    } catch {
      toast.show("Couldn't complete the update. Try again from Diagnostics.", { tone: "danger" });
    } finally {
      setBusy(false);
    }
  }, [applyDaemon, banner, busy, prefs, toast, update]);

  if (!banner) return null;
  return (
    <SafeAreaView edges={Platform.OS === "ios" ? ["top"] : []}>
      <Banner
        compact
        tone="neutral"
        message={busy ? "Working on the update..." : bannerMessage(banner, appVersion)}
        actionLabel={busy ? undefined : bannerActionLabel(banner)}
        onAction={act}
        secondaryLabel={canSkip(banner) && !busy ? "Skip this version" : undefined}
        onSecondary={() => update(skip(prefs, banner.key))}
        onDismiss={busy ? undefined : () => update(snooze(prefs, banner.key, Date.now()))}
      />
    </SafeAreaView>
  );
}
