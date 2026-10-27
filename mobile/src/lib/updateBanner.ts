// Which single update banner to show, as a pure decision over what the app knows.
//
// Three things can be out of date: this desktop app, the `forge` CLI/daemon it talks to, and (after
// a self-install) the running process itself. Only one banner is shown at a time, most pressing
// first, and the user can skip a version for good or be reminded later.

import type { DesktopUpdateState } from "./updater";

/** The daemon's `GET /api/update` body (only what the decision needs). */
export interface DaemonUpdateInfo {
  current: string;
  latest: string | null;
  update_available: boolean;
  notes?: string;
  url?: string;
  method?: "binary" | "homebrew" | "cargo" | "aur" | "scoop";
  command?: string | null;
  can_self_update: boolean;
  busy_sessions?: number;
  service_managed?: boolean;
  /** The daemon updates itself when idle; the desktop then only informs. Absent = false. */
  auto_update?: boolean;
}

/** What the desktop shell allows natively (`desktop_update_policy`). */
export interface DesktopUpdatePolicy {
  /** False when FORGE_NO_UPDATE_CHECK is set: no update lookups of any kind. */
  check: boolean;
  /** False when auto-install is off (opt-out env vars, or an install that cannot replace itself). */
  auto: boolean;
}

export const DEFAULT_POLICY: DesktopUpdatePolicy = { check: true, auto: true };

/** Progress of the desktop-driven daemon update, kept by the banner host. */
export type DaemonAutoState =
  | { phase: "running"; version: string }
  | { phase: "done"; version: string; restart: string }
  | { phase: "failed"; version: string };

export interface UpdatePrefs {
  /** Banner keys the user chose to skip for good, e.g. `app:2.24.0`. */
  skipped: string[];
  /** Banner key -> epoch ms before which it stays hidden ("remind me later"). */
  snoozedUntil: Record<string, number>;
}

export const EMPTY_PREFS: UpdatePrefs = { skipped: [], snoozedUntil: {} };

export const REMIND_LATER_MS = 24 * 3600 * 1000;

export interface BannerFacts {
  /** True inside the Tauri desktop shell: only there can anything be installed. */
  isDesktop: boolean;
  desktop: Pick<DesktopUpdateState, "phase" | "availableVersion" | "selfInstall" | "releaseUrl">;
  /** Version a self-install already put on disk, awaiting a restart. */
  installedVersion: string | null;
  daemon: DaemonUpdateInfo | null;
  /** A cached `update_available` from `/api/identity` when the full endpoint is unavailable. */
  identityLatest?: string | null;
  /** Set while/after the desktop auto-applies the daemon update. */
  daemonAuto?: DaemonAutoState | null;
  prefs: UpdatePrefs;
  now: number;
}

export type UpdateBanner =
  | { kind: "app-restart"; key: string; version: string }
  | { kind: "app-install"; key: string; version: string }
  | { kind: "app-download"; key: string; version: string; url: string | null }
  | { kind: "daemon-update"; key: string; version: string; command: string | null; installable: boolean }
  | { kind: "daemon-notice"; key: string; version: string }
  | { kind: "daemon-auto"; key: string; version: string; state: DaemonAutoState }
  | { kind: "daemon-self-updating"; key: string; version: string };

export function appKey(version: string): string {
  return `app:${version}`;
}

export function daemonKey(version: string): string {
  return `daemon:${version.replace(/^v/, "")}`;
}

export function isHidden(prefs: UpdatePrefs, key: string, now: number): boolean {
  if (prefs.skipped.includes(key)) return true;
  const until = prefs.snoozedUntil[key];
  return typeof until === "number" && until > now;
}

export function chooseUpdateBanner(facts: BannerFacts): UpdateBanner | null {
  const { prefs, now } = facts;
  if (facts.isDesktop && facts.installedVersion) {
    const key = `restart:${facts.installedVersion}`;
    if (!isHidden(prefs, key, now)) {
      return { kind: "app-restart", key, version: facts.installedVersion };
    }
  }
  const { phase, availableVersion } = facts.desktop;
  if (facts.isDesktop && phase === "available" && availableVersion) {
    const key = appKey(availableVersion);
    if (!isHidden(prefs, key, now)) {
      return facts.desktop.selfInstall
        ? { kind: "app-install", key, version: availableVersion }
        : { kind: "app-download", key, version: availableVersion, url: facts.desktop.releaseUrl };
    }
  }
  const daemon = facts.daemon;
  const auto = facts.daemonAuto;
  if (facts.isDesktop && auto) {
    const key = `${daemonKey(auto.version)}:auto`;
    if (!isHidden(prefs, key, now)) return { kind: "daemon-auto", key, version: auto.version, state: auto };
  }
  const latest = daemon?.update_available ? daemon.latest : (facts.identityLatest ?? null);
  if (latest) {
    const key = daemonKey(latest);
    if (!isHidden(prefs, key, now)) {
      const version = latest.replace(/^v/, "");
      if (facts.isDesktop && daemon?.update_available && daemon.auto_update) {
        return { kind: "daemon-self-updating", key, version };
      }
      if (facts.isDesktop && daemon?.update_available) {
        return {
          kind: "daemon-update",
          key,
          version,
          command: daemon.command ?? null,
          installable: daemon.can_self_update,
        };
      }
      return { kind: "daemon-notice", key, version };
    }
  }
  return null;
}

export function bannerMessage(banner: UpdateBanner, appVersion: string): string {
  switch (banner.kind) {
    case "app-restart":
      return `Updated to Forge ${banner.version} - restart to apply.`;
    case "app-install":
      return `Forge Desktop ${banner.version} is available (you have ${appVersion}).`;
    case "app-download":
      return `Forge Desktop ${banner.version} is available (you have ${appVersion}). This install updates from the release page.`;
    case "daemon-update":
      return banner.installable
        ? `Forge (CLI) ${banner.version} is available on this computer.`
        : `Forge (CLI) ${banner.version} is available. Update with: ${banner.command ?? "forge update"}`;
    case "daemon-notice":
      return `Forge ${banner.version} is available on the host. Update it with \`forge update\` there.`;
    case "daemon-self-updating":
      return `Forge (CLI) ${banner.version} is available. It updates itself when no session is busy.`;
    case "daemon-auto":
      if (banner.state.phase === "running") return `Updating Forge (CLI) to ${banner.version}...`;
      if (banner.state.phase === "failed") {
        return `Couldn't update Forge (CLI) to ${banner.version} automatically. Run \`forge update\` on this computer.`;
      }
      return banner.state.restart === "manual"
        ? `Forge (CLI) updated to ${banner.version}. Restart \`forge serve\` to use it.`
        : `Forge (CLI) updated to ${banner.version}. The service restarts when no session is busy.`;
  }
}

export function bannerActionLabel(banner: UpdateBanner): string | undefined {
  switch (banner.kind) {
    case "app-restart":
      return "Restart";
    case "app-install":
      return "Install";
    case "app-download":
      return "Open release page";
    case "daemon-update":
      return banner.installable ? "Update" : undefined;
    case "daemon-notice":
    case "daemon-self-updating":
    case "daemon-auto":
      return undefined;
  }
}

/** A restart banner only offers "later"; every other banner can also skip the version. */
export function canSkip(banner: UpdateBanner): boolean {
  return banner.kind !== "app-restart" && banner.kind !== "daemon-auto";
}

export function snooze(prefs: UpdatePrefs, key: string, now: number): UpdatePrefs {
  return { ...prefs, snoozedUntil: { ...prefs.snoozedUntil, [key]: now + REMIND_LATER_MS } };
}

export function skip(prefs: UpdatePrefs, key: string): UpdatePrefs {
  return prefs.skipped.includes(key) ? prefs : { ...prefs, skipped: [...prefs.skipped, key] };
}

const STORAGE_KEY = "forge.updateBanner.v1";

export function parsePrefs(raw: string | null): UpdatePrefs {
  if (!raw) return EMPTY_PREFS;
  try {
    const parsed = JSON.parse(raw) as Partial<UpdatePrefs> | null;
    const skipped = Array.isArray(parsed?.skipped)
      ? parsed.skipped.filter((s): s is string => typeof s === "string")
      : [];
    const snoozedUntil: Record<string, number> = {};
    for (const [k, v] of Object.entries(parsed?.snoozedUntil ?? {})) {
      if (typeof v === "number") snoozedUntil[k] = v;
    }
    return { skipped, snoozedUntil };
  } catch {
    return EMPTY_PREFS;
  }
}

export function loadPrefs(): UpdatePrefs {
  try {
    return parsePrefs(globalThis.localStorage?.getItem(STORAGE_KEY) ?? null);
  } catch {
    return EMPTY_PREFS;
  }
}

export function savePrefs(prefs: UpdatePrefs): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // A banner shown once more beats a failed write.
  }
}

/** A loopback daemon: the only kind the desktop may ask to replace its own binary. */
export function isLoopbackBase(baseUrl: string | null): boolean {
  if (!baseUrl) return false;
  try {
    const host = new URL(baseUrl).hostname.replace(/^\[|\]$/g, "");
    return host === "127.0.0.1" || host === "localhost" || host === "::1";
  } catch {
    return false;
  }
}

export interface DaemonAutoFacts {
  isDesktop: boolean;
  policy: DesktopUpdatePolicy;
  baseUrl: string | null;
  daemon: DaemonUpdateInfo | null;
  /** Versions already attempted in this app run (success or failure): never twice. */
  attempted: ReadonlySet<string>;
}

/**
 * The version to apply to the daemon right now, or null. Everything must line up: a desktop shell
 * that allows auto-install, a loopback daemon that can replace itself, no session mid-turn, a
 * daemon that is not already updating itself, and a version not yet tried in this run.
 */
export function daemonAutoTarget(facts: DaemonAutoFacts): string | null {
  const { daemon } = facts;
  if (!facts.isDesktop || !facts.policy.check || !facts.policy.auto) return null;
  if (!daemon?.update_available || !daemon.can_self_update || daemon.auto_update) return null;
  if (!isLoopbackBase(facts.baseUrl) || (daemon.busy_sessions ?? 0) > 0) return null;
  const version = daemon.latest?.replace(/^v/, "");
  if (!version || facts.attempted.has(version)) return null;
  return version;
}
