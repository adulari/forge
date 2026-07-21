export function resolveAppVersion(
  tauri: boolean,
  tauriVersion: string | null | undefined,
  expoVersion: string | null | undefined,
): string {
  if (tauri && tauriVersion?.trim()) return tauriVersion.trim();
  return expoVersion?.trim() || "—";
}
