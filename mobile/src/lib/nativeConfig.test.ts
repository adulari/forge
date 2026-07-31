import { describe, expect, it } from "vitest";

import config from "../../app.config";

describe("native release config", () => {
  it("blocks React Native's overlay permission from Android manifests", () => {
    expect(config.android?.blockedPermissions).toContain(
      "android.permission.SYSTEM_ALERT_WINDOW",
    );
  });
});
