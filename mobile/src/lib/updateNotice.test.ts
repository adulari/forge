import { describe, expect, it } from "vitest";

import { updateNotice } from "./updateNotice";

const base = {
  updateId: "abc",
  appVersion: "1.0.1",
  lastSeenUpdateId: "abc",
  lastSeenAppVersion: "1.0.1",
};

describe("updateNotice", () => {
  it("says nothing when nothing changed", () => {
    expect(updateNotice(base)).toBeNull();
  });

  it("stays silent on a fresh install", () => {
    // Greeting a first launch with "what's new" is noise — there is no version it came from.
    expect(
      updateNotice({ ...base, lastSeenUpdateId: null, lastSeenAppVersion: null }),
    ).toBeNull();
  });

  it("reports an OTA when only the update id moved", () => {
    expect(updateNotice({ ...base, updateId: "def" })).toEqual({ kind: "ota", appVersion: "1.0.1" });
  });

  it("reports the app when the native version moved", () => {
    expect(updateNotice({ ...base, appVersion: "1.0.2" })).toEqual({
      kind: "app",
      appVersion: "1.0.2",
    });
  });

  it("calls a build that also brought an OTA an app update, not two events", () => {
    // A build ships and the first OTA lands on it. Both ids differ, but the user experienced one
    // update, so it must not produce two dialogs or the wrong headline.
    const notice = updateNotice({ ...base, appVersion: "1.0.2", updateId: "def" });
    expect(notice).toEqual({ kind: "app", appVersion: "1.0.2" });
  });

  it("treats a rollback to the embedded bundle as an update", () => {
    // Going from an OTA back to null is what a rollback looks like from here, and the code running
    // under the user did change — staying silent would be the lie.
    expect(updateNotice({ ...base, updateId: null })).toEqual({ kind: "ota", appVersion: "1.0.1" });
  });

  it("does not fire on the launch after a first launch recorded its starting point", () => {
    // The first launch records where it started; the next one must compare against that and find
    // nothing, rather than treating the newly-written record as a change.
    const recorded = { ...base, lastSeenUpdateId: base.updateId, lastSeenAppVersion: base.appVersion };
    expect(updateNotice(recorded)).toBeNull();
  });
});
