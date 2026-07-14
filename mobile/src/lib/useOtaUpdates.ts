// EAS Update (OTA) client: on cold start and whenever the app returns to the
// foreground, ask expo-updates whether a newer JS bundle is published for this
// runtimeVersion; if so, fetch + reload. No-op in dev and when updates are
// disabled (e.g. the Tauri desktop/web build, which has no expo-updates native
// module). Errors are swallowed so an offline/failed check never crashes the app.
import * as Updates from "expo-updates";
import { useEffect } from "react";
import { AppState, type AppStateStatus } from "react-native";

async function checkAndApplyUpdate(): Promise<void> {
  if (!Updates.isEnabled || __DEV__) return;
  const check = await Updates.checkForUpdateAsync();
  if (!check.isAvailable) return;
  await Updates.fetchUpdateAsync();
  await Updates.reloadAsync();
}

export function useOtaUpdates(): void {
  useEffect(() => {
    // Best-effort check on launch — fire-and-forget so it never blocks first paint.
    checkAndApplyUpdate().catch(() => undefined);

    const subscription = AppState.addEventListener("change", (state: AppStateStatus) => {
      if (state === "active") checkAndApplyUpdate().catch(() => undefined);
    });
    return () => subscription.remove();
  }, []);
}
