import Constants from "expo-constants";
import { useEffect, useState } from "react";

import { isTauri } from "./platform";
import { resolveAppVersion } from "./appVersionCore";

export { resolveAppVersion } from "./appVersionCore";

/** Runtime version shown by Settings: the signed Tauri bundle on desktop, Expo elsewhere. */
export function useAppVersion(): string {
  const expoVersion = Constants.expoConfig?.version ?? null;
  const [version, setVersion] = useState(() => resolveAppVersion(isTauri, null, expoVersion));

  useEffect(() => {
    if (!isTauri) return;
    let active = true;
    void import("@tauri-apps/api/app")
      .then(({ getVersion }) => getVersion())
      .then((tauriVersion) => {
        if (active) setVersion(resolveAppVersion(true, tauriVersion, expoVersion));
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [expoVersion]);

  return version;
}
