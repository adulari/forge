import type { ChangelogRelease } from "./api";

/**
 * Changelog bullets open with a bold one-line summary followed by the detail:
 * `**Auto-edit ran every shell command.** It now asks ...`. The update sheet shows the summary
 * and keeps the detail behind a tap; a bullet without that shape is all summary.
 */
export function splitEntry(text: string): { lead: string; detail: string } {
  const match = /^\*\*(.+?)\*\*\s*(.*)$/s.exec(text.trim());
  if (!match) return { lead: text.trim(), detail: "" };
  return { lead: match[1].trim(), detail: match[2].trim() };
}

/**
 * The desktop app shares its version with the daemon, so it can show the notes for exactly the
 * build it installed. An older daemon does not have that section yet, and its newest one describes
 * a different release, so the caller gets null and links to the published notes instead.
 * Mobile builds have their own version line and keep showing the daemon's newest release.
 */
export function releaseForApp(
  releases: ChangelogRelease[] | undefined,
  appVersion: string,
  matchVersion: boolean,
): ChangelogRelease | null {
  if (!releases?.length) return null;
  if (!matchVersion) return releases[0];
  return releases.find((release) => release.version === appVersion) ?? null;
}

export function releaseNotesUrl(version: string): string {
  return `https://github.com/adulari/forge/releases/tag/v${version}`;
}
