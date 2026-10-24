export function resolveAppVersion(
  tauri: boolean,
  tauriVersion: string | null | undefined,
  expoVersion: string | null | undefined,
): string {
  if (tauri && tauriVersion?.trim()) return tauriVersion.trim();
  return expoVersion?.trim() || "—";
}

/**
 * The version to compare against the last-seen build, or null while it is not known yet. Unlike
 * `resolveAppVersion` there is no fallback to the Expo client version on desktop: the bundle
 * version arrives asynchronously, and comparing the client version first and the bundle version a
 * moment later recorded one and then saw the other, so the "app updated" sheet came back on
 * every launch.
 */
export function settledAppVersion(
  tauri: boolean,
  tauriVersion: string | null | undefined,
  expoVersion: string | null | undefined,
): string | null {
  if (tauri) return tauriVersion?.trim() || null;
  return expoVersion?.trim() || null;
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
