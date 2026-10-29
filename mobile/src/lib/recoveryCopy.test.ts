import { describe, expect, it } from "vitest";

import { backupLabel } from "./recoveryCopy";

describe("backupLabel", () => {
  it("names the schema the copy was taken before, with its age and size", () => {
    const label = backupLabel(
      { name: "forge.db.pre-v46.bak", before_version: 46, bytes: 5 * 1024 * 1024, modified: 1_700_000_000 },
      "en-GB",
    );
    expect(label.title).toBe("Before schema v46");
    expect(label.subtitle).toContain("5.0 MB");
    expect(label.subtitle).toContain("2023");
  });
});
