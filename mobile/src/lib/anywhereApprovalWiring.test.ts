// Guards the one path that re-enrolls a locked-out device. The approval screen once ran on
// MockAnywhereClient: it accepted a challenge, said "Approved", and called nothing — so an
// account whose only terminal had lost its refresh token could not be recovered from the phone
// at all, and the Recovery Kit was the sole way back in. The hub's inbox is text-only by design
// (the service never lists the challenge), so this screen is the whole approval surface.
//
// Source-text assertions rather than a render test: the regression is which client the screen is
// wired to, and a render test with a mocked provider would pass against either wiring. It lives
// in lib/ because every file under src/app is itself a route.
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const appDir = join(dirname(fileURLToPath(import.meta.url)), "..", "app");
const read = (name: string) => readFileSync(join(appDir, "anywhere", name), "utf8");

describe("Anywhere device approval", () => {
  it("approves through the real provider, never the mock client", () => {
    const source = read("pair.tsx");
    expect(source).toContain('from "../../lib/AnywhereProvider"');
    expect(source).not.toContain('from "../../lib/anywhere/store"');
    expect(source).toContain("approvePairing(");
    expect(source).toContain("inspectPairing(");
  });

  it("shows the safety code before the approve action", () => {
    const source = read("pair.tsx");
    expect(source.indexOf("safetyCode")).toBeGreaterThan(-1);
    expect(source.indexOf("safetyCode")).toBeLessThan(source.indexOf('label="Approve"'));
  });

  it("reaches that screen from the hub's approval inbox", () => {
    expect(read("index.tsx")).toContain('router.push("/anywhere/pair")');
  });
});
