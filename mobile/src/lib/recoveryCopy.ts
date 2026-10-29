import type { RecoveryBackup } from "./api";
import { formatBytes } from "./anywhere/format";

/** The two lines a backup row shows on the recovery screen. */
export function backupLabel(backup: RecoveryBackup, locale?: string): { title: string; subtitle: string } {
  const when = new Date(backup.modified * 1000).toLocaleString(locale);
  return {
    title: `Before schema v${backup.before_version}`,
    subtitle: `${when} · ${formatBytes(backup.bytes)}`,
  };
}
