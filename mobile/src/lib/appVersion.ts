import Constants from "expo-constants";
import { useEffect, useState } from "react";

import { isTauri } from "./platform";
import { formatVersionMeta, resolveAppVersion } from "./appVersionCore";

export { formatVersionMeta, resolveAppVersion } from "./appVersionCore";

/**
 * Version stamped into the installed Tauri bundle — the authoritative desktop release number, and
 * the same value the updater compares against `latest.json`. `null` off desktop, and until the
 * async lookup lands.
 */
function useDesktopBundleVersion(): string | null {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri) return;
    let active = true;
    void import("@tauri-apps/api/app")
      .then(({ getVersion }) => getVersion())
      .then((bundleVersion) => {
        if (active) setVersion(bundleVersion);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, []);

  return version;
}

/** Runtime version: the signed Tauri bundle on desktop, the shared Expo client elsewhere. */
export function useAppVersion(): string {
  const bundleVersion = useDesktopBundleVersion();
  return resolveAppVersion(isTauri, bundleVersion, Constants.expoConfig?.version ?? null);
}

/** Settings' `v… · protocol v…` meta line, with the desktop bundle labelled separately. */
export function useVersionMeta(protocolVersion: number): string {
  const bundleVersion = useDesktopBundleVersion();
  return formatVersionMeta(
    isTauri,
    bundleVersion,
    Constants.expoConfig?.version ?? null,
    protocolVersion,
  );
}
