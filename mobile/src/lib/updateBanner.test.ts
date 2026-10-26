import { describe, expect, it } from "vitest";
import {
  EMPTY_PREFS,
  REMIND_LATER_MS,
  bannerActionLabel,
  chooseUpdateBanner,
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
