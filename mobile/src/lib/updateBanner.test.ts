import { describe, expect, it } from "vitest";
import {
  EMPTY_PREFS,
  REMIND_LATER_MS,
  bannerActionLabel,
  bannerMessage,
  chooseUpdateBanner,
  daemonAutoTarget,
  isLoopbackBase,
  parsePrefs,
  skip,
  snooze,
  type BannerFacts,
} from "./updateBanner";

const base: BannerFacts = {
  isDesktop: true,
  desktop: { phase: "idle", availableVersion: null, selfInstall: true, releaseUrl: null },
  installedVersion: null,
  daemon: null,
  prefs: EMPTY_PREFS,
  now: 1_000,
};

const appUpdate = { phase: "available" as const, availableVersion: "3.0.0", selfInstall: true, releaseUrl: "u" };
const daemon = { current: "2.0.0", latest: "2.1.0", update_available: true, can_self_update: true, command: null };

describe("chooseUpdateBanner", () => {
  it("shows nothing when everything is current", () => {
    expect(chooseUpdateBanner(base)).toBeNull();
  });

  it("offers Install for a self-installing app and the release page otherwise", () => {
    const a = chooseUpdateBanner({ ...base, desktop: appUpdate });
    expect(a).toMatchObject({ kind: "app-install", version: "3.0.0" });
    const b = chooseUpdateBanner({ ...base, desktop: { ...appUpdate, selfInstall: false } });
    expect(b).toMatchObject({ kind: "app-download", url: "u" });
    expect(bannerActionLabel(b!)).toBe("Open release page");
  });

  it("puts restart before app update before daemon update", () => {
    const all = { ...base, desktop: appUpdate, daemon, installedVersion: "2.9.0" };
    expect(chooseUpdateBanner(all)?.kind).toBe("app-restart");
    expect(chooseUpdateBanner({ ...all, installedVersion: null })?.kind).toBe("app-install");
    expect(chooseUpdateBanner({ ...all, installedVersion: null, desktop: base.desktop })?.kind).toBe("daemon-update");
  });

  it("skips a version for good but announces the next one", () => {
    const prefs = skip(EMPTY_PREFS, "app:3.0.0");
    expect(chooseUpdateBanner({ ...base, desktop: appUpdate, prefs })).toBeNull();
    const next = chooseUpdateBanner({ ...base, desktop: { ...appUpdate, availableVersion: "3.0.1" }, prefs });
    expect(next).toMatchObject({ version: "3.0.1" });
  });

  it("hides a snoozed banner until the reminder is due", () => {
    const prefs = snooze(EMPTY_PREFS, "daemon:2.1.0", base.now);
    expect(chooseUpdateBanner({ ...base, daemon, prefs })).toBeNull();
    expect(chooseUpdateBanner({ ...base, daemon, prefs, now: base.now + REMIND_LATER_MS + 1 })).not.toBeNull();
  });

  it("never offers to install the daemon update off desktop or when it cannot self-update", () => {
    expect(chooseUpdateBanner({ ...base, isDesktop: false, daemon })?.kind).toBe("daemon-notice");
    const manual = chooseUpdateBanner({
      ...base,
      daemon: { ...daemon, can_self_update: false, command: "brew upgrade forge" },
    });
    expect(manual).toMatchObject({ kind: "daemon-update", installable: false, command: "brew upgrade forge" });
    expect(bannerActionLabel(manual!)).toBeUndefined();
  });

  it("falls back to the cheap identity hint for phones", () => {
    const b = chooseUpdateBanner({ ...base, isDesktop: false, identityLatest: "2.5.0" });
    expect(b).toMatchObject({ kind: "daemon-notice", version: "2.5.0" });
  });

  it("tolerates corrupt stored prefs", () => {
    expect(parsePrefs("{nope")).toEqual(EMPTY_PREFS);
    expect(parsePrefs('{"skipped":["a",1],"snoozedUntil":{"a":5,"b":"x"}}')).toEqual({
      skipped: ["a"],
      snoozedUntil: { a: 5 },
    });
  });
});

describe("daemon auto-update", () => {
  const facts = {
    isDesktop: true,
    policy: { check: true, auto: true },
    baseUrl: "http://127.0.0.1:7420/tok",
    daemon: { ...daemon, busy_sessions: 0 },
    attempted: new Set<string>(),
  };

  it("targets the daemon's latest version when everything lines up", () => {
    expect(daemonAutoTarget(facts)).toBe("2.1.0");
  });

  it("waits while any session is busy and never for a daemon that cannot replace itself", () => {
    expect(daemonAutoTarget({ ...facts, daemon: { ...facts.daemon, busy_sessions: 1 } })).toBeNull();
    expect(daemonAutoTarget({ ...facts, daemon: { ...facts.daemon, can_self_update: false } })).toBeNull();
    expect(daemonAutoTarget({ ...facts, daemon: { ...facts.daemon, update_available: false } })).toBeNull();
  });

  it("leaves a daemon that updates itself alone", () => {
    const self = { ...facts, daemon: { ...facts.daemon, auto_update: true } };
    expect(daemonAutoTarget(self)).toBeNull();
    const banner = chooseUpdateBanner({ ...base, daemon: self.daemon });
    expect(banner?.kind).toBe("daemon-self-updating");
    expect(bannerActionLabel(banner!)).toBeUndefined();
  });

  it("is off when policy forbids it, off the desktop shell, or for a non-loopback daemon", () => {
    expect(daemonAutoTarget({ ...facts, policy: { check: true, auto: false } })).toBeNull();
    expect(daemonAutoTarget({ ...facts, policy: { check: false, auto: true } })).toBeNull();
    expect(daemonAutoTarget({ ...facts, isDesktop: false })).toBeNull();
    expect(daemonAutoTarget({ ...facts, baseUrl: "https://192.168.1.5:7420/tok" })).toBeNull();
    expect(daemonAutoTarget({ ...facts, baseUrl: null })).toBeNull();
  });

  it("tries each version once per app run", () => {
    expect(daemonAutoTarget({ ...facts, attempted: new Set(["2.1.0"]) })).toBeNull();
    expect(daemonAutoTarget({ ...facts, daemon: { ...facts.daemon, latest: "v2.2.0" }, attempted: new Set(["2.1.0"]) })).toBe("2.2.0");
  });

  it("recognises loopback hosts only", () => {
    expect(isLoopbackBase("http://localhost:7420/t")).toBe(true);
    expect(isLoopbackBase("http://[::1]:7420/t")).toBe(true);
    expect(isLoopbackBase("http://example.com/t")).toBe(false);
    expect(isLoopbackBase("not a url")).toBe(false);
  });

  it("shows progress and the result as a banner with no required action", () => {
    const running = chooseUpdateBanner({ ...base, daemonAuto: { phase: "running", version: "2.1.0" } })!;
    expect(bannerMessage(running, "1")).toBe("Updating Forge (CLI) to 2.1.0...");
    expect(bannerActionLabel(running)).toBeUndefined();
    const done = chooseUpdateBanner({ ...base, daemonAuto: { phase: "done", version: "2.1.0", restart: "scheduled" } })!;
    expect(bannerMessage(done, "1")).toContain("restarts when no session is busy");
    const failed = chooseUpdateBanner({ ...base, daemonAuto: { phase: "failed", version: "2.1.0" } })!;
    expect(bannerMessage(failed, "1")).toContain("forge update");
  });
});
