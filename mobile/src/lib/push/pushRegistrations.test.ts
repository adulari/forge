import { describe, expect, it } from "vitest";

import { pushRegistrationDue, withRegistration, withoutRegistration } from "./pushRegistrations";

const keys = { p256dh: "pub", auth: "auth" };

describe("push registration ledger", () => {
  it("is due for a host never registered", () => {
    expect(pushRegistrationDue({}, "http://a", "tok", keys)).toBe(true);
  });

  it("is settled once the same token and key are registered", () => {
    const ledger = withRegistration({}, "http://a", "tok", keys);
    expect(pushRegistrationDue(ledger, "http://a", "tok", keys)).toBe(false);
    expect(pushRegistrationDue(ledger, "http://b", "tok", keys)).toBe(true);
  });

  it("is due again when the token rotates or the key changes", () => {
    const ledger = withRegistration({}, "http://a", "tok", keys);
    expect(pushRegistrationDue(ledger, "http://a", "tok2", keys)).toBe(true);
    expect(pushRegistrationDue(ledger, "http://a", "tok", { ...keys, p256dh: "other" })).toBe(true);
  });

  it("upgrades a keyless registration once a key is available", () => {
    const ledger = withRegistration({}, "http://a", "tok", null);
    expect(pushRegistrationDue(ledger, "http://a", "tok", null)).toBe(false);
    expect(pushRegistrationDue(ledger, "http://a", "tok", keys)).toBe(true);
  });

  it("forgets a host on unsubscribe without touching the others", () => {
    const ledger = withRegistration(withRegistration({}, "http://a", "t", keys), "http://b", "t", keys);
    expect(withoutRegistration(ledger, "http://a")).toEqual({ "http://b": "t|pub" });
  });
});
