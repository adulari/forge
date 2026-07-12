import { isTauri } from "./platform";

export type DesktopUpdate = { version: string; body?: string | null; install: () => Promise<void> };

export async function checkForDesktopUpdate(): Promise<DesktopUpdate | null> {
  if (!isTauri) return null;
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    body: update.body,
    install: async () => {
      await update.downloadAndInstall();
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    },
  };
}
