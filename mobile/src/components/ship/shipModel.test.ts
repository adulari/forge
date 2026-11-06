import { describe, expect, it } from "vitest";

import type { ShipResponse } from "../../lib/apiShip";
import { shipFinished, shipHeadline, shipRequestBody, shipRows } from "./shipModel";

const base: ShipResponse = {
  ok: true,
  steps: [
    { step: "branch", status: "done", detail: "shipping `feat`" },
    { step: "commit", status: "done", detail: "abc1234 feat: cart" },
    { step: "push", status: "done", detail: "pushed `feat`" },
    { step: "pr", status: "done", detail: "PR #9 opened: feat: cart" },
    { step: "watch", status: "done", detail: "watching PR #9 in merge mode (squash)" },
  ],
  branch: "feat",
  base: "main",
  commit: { sha: "abc1234", message: "feat: cart" },
  pr: { number: 9, url: "https://github.com/acme/shop/pull/9", title: "feat: cart", existing: false },
  watch: null,
  refused: null,
};

describe("shipRows", () => {
  it("is four pending rows before anything runs", () => {
    const rows = shipRows(null, false);
    expect(rows.map((r) => r.key)).toEqual(["commit", "push", "pr", "watch"]);
    expect(rows.every((r) => r.state === "pending")).toBe(true);
  });

  it("marks the first row active while the request is in flight", () => {
    expect(shipRows(null, true).map((r) => r.state)).toEqual(["active", "pending", "pending", "pending"]);
  });

  it("shows every step done with the daemon's detail, leaving the branch check out", () => {
    const rows = shipRows(base, false);
    expect(rows.map((r) => r.state)).toEqual(["done", "done", "done", "done"]);
    expect(rows[2].detail).toBe("PR #9 opened: feat: cart");
  });

  it("blocks the rows after a refusal and keeps the refusal on its own row", () => {
    const refused: ShipResponse = {
      ...base,
      ok: false,
      steps: [
        { step: "branch", status: "done", detail: "" },
        { step: "commit", status: "done", detail: "abc1234 feat: cart" },
        { step: "pr", status: "refused", detail: "not write access" },
      ],
      refused: { step: "pr", reason: "not write access" },
    };
    expect(shipRows(refused, false).map((r) => r.state)).toEqual(["done", "blocked", "refused", "blocked"]);
  });

  it("blocks every row when the branch check refuses", () => {
    const refused: ShipResponse = {
      ...base,
      ok: false,
      steps: [{ step: "branch", status: "refused", detail: "create a branch first" }],
      refused: { step: "branch", reason: "create a branch first" },
    };
    expect(shipRows(refused, false).every((r) => r.state === "blocked")).toBe(true);
  });

  it("shows a skipped commit as skipped", () => {
    const skipped: ShipResponse = {
      ...base,
      steps: base.steps.map((s) => (s.step === "commit" ? { ...s, status: "skipped" as const } : s)),
    };
    expect(shipRows(skipped, false)[0].state).toBe("skipped");
  });
});

describe("shipHeadline", () => {
  it("names the PR on success", () => {
    expect(shipHeadline(base, false)).toBe("PR #9 is open and being watched. Forge merges it when checks pass.");
  });

  it("names the stopping step and the reason", () => {
    const refused: ShipResponse = {
      ...base,
      ok: false,
      refused: { step: "branch", reason: "on `main`, the base branch; create a branch first" },
    };
    expect(shipHeadline(refused, false)).toBe(
      "Stopped before committing: on `main`, the base branch; create a branch first",
    );
    expect(shipHeadline({ ...refused, refused: { step: "pr", reason: "no write access" } }, false)).toBe(
      "Stopped at open pull request: no write access",
    );
  });

  it("describes the idle and running sheet", () => {
    expect(shipHeadline(null, true)).toBe("Shipping…");
    expect(shipHeadline(null, false)).toContain("Commit, push");
  });
});

describe("shipRequestBody", () => {
  it("sends a title only when one was typed", () => {
    expect(shipRequestBody("   ")).toEqual({});
    expect(shipRequestBody("  Add   the cart ")).toEqual({ title: "Add the cart" });
  });
});

describe("shipFinished", () => {
  it("is true for a success or a refusal", () => {
    expect(shipFinished(null)).toBe(false);
    expect(shipFinished(base)).toBe(true);
    expect(shipFinished({ ...base, ok: false, refused: { step: "push", reason: "x" } })).toBe(true);
  });
});
