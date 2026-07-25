export function resolveAppVersion(
  tauri: boolean,
  tauriVersion: string | null | undefined,
  expoVersion: string | null | undefined,
): string {
  if (tauri && tauriVersion?.trim()) return tauriVersion.trim();
  return expoVersion?.trim() || "—";
}

/**
 * Settings meta line. On desktop the installed bundle version (what the updater compares, and what
 * the release artifacts are named for) and the shared client build are two different numbers, so
 * label both rather than printing one unqualified `v…` that reads as the application version
 * (#838). Every non-desktop surface keeps the single shared client version it already shows.
 */
export function formatVersionMeta(
  tauri: boolean,
  tauriVersion: string | null | undefined,
  expoVersion: string | null | undefined,
  protocolVersion: number,
): string {
  const bundleVersion = tauriVersion?.trim();
  const clientVersion = expoVersion?.trim();
  if (tauri && bundleVersion && clientVersion) {
    return `Desktop v${bundleVersion} · client v${clientVersion} · protocol v${protocolVersion}`;
  }
  // Until the async bundle lookup resolves — and if it never does — fall back to the unlabelled
  // shared client version instead of guessing which release is installed.
  return `v${resolveAppVersion(tauri, tauriVersion, expoVersion)} · protocol v${protocolVersion}`;
}
