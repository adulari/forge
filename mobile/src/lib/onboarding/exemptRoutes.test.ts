import { describe, expect, it } from "vitest";

import { isOnboardingExempt } from "./exemptRoutes";

describe("isOnboardingExempt", () => {
  it("keeps the import screen reachable from the first-run import step", () => {
    expect(isOnboardingExempt("/import")).toBe(true);
    expect(isOnboardingExempt("/onboarding")).toBe(true);
    expect(isOnboardingExempt("/onboarding/models")).toBe(true);
  });

  it("still redirects every other route while setup is open", () => {
    expect(isOnboardingExempt("/")).toBe(false);
    expect(isOnboardingExempt("/important")).toBe(false);
    expect(isOnboardingExempt("/settings")).toBe(false);
  });
});
