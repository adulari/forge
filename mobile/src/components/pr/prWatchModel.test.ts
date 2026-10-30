import { describe, expect, it } from "vitest";

import type { PrWatch } from "../../lib/apiPr";
import { isWatching, watchStatus } from "./prWatchModel";

const base: PrWatch = {
  session_id: "s",
  mode: "merge",
  method: "squash",
  state: "watching",
  blocked_reason: null,
  pr_number: 7,
  pr_url: null,
  head_sha: null,
  fix_turns: 0,
  last_checked: null,
  last_action: null,
  created_at: 0,
};

describe("watchStatus", () => {
  it("is quiet when nothing is armed", () => {
    expect(watchStatus(null).label).toBe("Not watching");
    expect(watchStatus({ ...base, mode: "off" }).label).toBe("Not watching");
  });

  it("names the mode while watching and surfaces what it last did", () => {
    expect(watchStatus(base).label).toBe("Watching to merge");
    expect(watchStatus({ ...base, mode: "fix", last_action: "fix turn 1 sent" })).toMatchObject({
      label: "Watching",
      detail: "fix turn 1 sent",
    });
  });

  it("shows a waiting reason as a warning and a stop as blocked", () => {
    expect(watchStatus({ ...base, blocked_reason: "waiting for a required review or check" }).tone).toBe("warn");
    expect(watchStatus({ ...base, state: "blocked", blocked_reason: "refused a force push" })).toMatchObject({
      label: "Blocked",
      tone: "danger",
      detail: "refused a force push",
    });
  });

  it("reports a merge", () => {
    expect(watchStatus({ ...base, state: "merged" }).detail).toContain("marked done");
  });
});

describe("isWatching", () => {
  it("is true only for an armed, unfinished watch", () => {
    expect(isWatching(base)).toBe(true);
    expect(isWatching({ ...base, state: "merged" })).toBe(false);
    expect(isWatching({ ...base, mode: "off" })).toBe(false);
    expect(isWatching(null)).toBe(false);
  });
});
