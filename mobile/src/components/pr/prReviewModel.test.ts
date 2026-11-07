import { describe, expect, it } from "vitest";

import type { PrQueueRow, PrReview, PrWatch } from "../../lib/apiPr";
import { rowStatus } from "./mergeQueueModel";
import { blockingFindings, canMergeAnyway, findingPlace, numberedBlocking, reviewLabel } from "./prReviewModel";
import { watchStatus } from "./prWatchModel";

const finding = (over = {}) => ({
  severity: "high",
  file: "src/a.rs",
  line: 12,
  title: "unwrap on input",
  why: "panics",
  blocking: true,
  ...over,
});
const review = (over: Partial<PrReview> = {}): PrReview => ({
  head_sha: "abc",
  verdict: "findings",
  findings: [finding(), finding({ line: null, blocking: false, severity: "low" })],
  model: "m",
  error: null,
  fix_sent: true,
  rounds: 1,
  ...over,
});
const watch = (r: PrReview | null): PrWatch => ({
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
  review: r,
});

describe("reviewLabel", () => {
  it("spells the daemon's row text", () => {
    expect(reviewLabel(review())).toBe("review: 1 confirmed, fixing");
    expect(reviewLabel(review({ fix_sent: false }))).toBe("review: 1 confirmed");
    expect(reviewLabel(review({ verdict: "clean", findings: [] }))).toBe("review clean");
    expect(reviewLabel(review({ verdict: "error" }))).toBe("review failed, will retry");
    expect(reviewLabel(null)).toBeNull();
  });
  it("lists only the findings that hold the merge, with file:line", () => {
    const blocking = blockingFindings(review());
    expect(blocking).toHaveLength(1);
    expect(findingPlace(blocking[0])).toBe("src/a.rs:12");
    expect(findingPlace(finding({ line: null }))).toBe("src/a.rs");
  });
});

describe("the review on the watch and queue rows", () => {
  it("shows in the PR Watch status and turns it to a warning", () => {
    const status = watchStatus(watch(review()));
    expect(status.detail).toContain("review: 1 confirmed, fixing");
    expect(status.tone).toBe("warn");
    expect(watchStatus(watch(review({ verdict: "clean", findings: [] }))).detail).toBe("review clean");
    expect(watchStatus(watch(null)).detail).toBeNull();
  });
  it("shows on a waiting queue row", () => {
    const row: PrQueueRow = {
      session_id: "s",
      pr_number: 7,
      pr_url: null,
      position: 1,
      state: "head",
      reason: "waiting for the session to push",
      pushes: 0,
      last_update: null,
      review: "review: 2 confirmed, fixing",
    };
    const status = rowStatus(row);
    expect(status.detail).toBe("review: 2 confirmed, fixing - waiting for the session to push");
    expect(status.tone).toBe("warn");
    expect(rowStatus({ ...row, review: undefined }).detail).toBe("waiting for the session to push");
  });
});

describe("owner overrides", () => {
  it("numbers blocking findings by their place in the whole list", () => {
    const r = review({ findings: [finding({ blocking: false }), finding({ title: "second" })] });
    expect(numberedBlocking(r).map((x) => x.n)).toEqual([2]);
  });
  it("offers merge anyway only while a review holds the merge, and not twice", () => {
    expect(canMergeAnyway(review())).toBe(true);
    expect(canMergeAnyway(review({ verdict: "running", findings: [] }))).toBe(true);
    expect(canMergeAnyway(review({ verdict: "clean", findings: [] }))).toBe(false);
    expect(canMergeAnyway(review({ merge_anyway: { by: "app", at: 1 } }))).toBe(false);
    expect(canMergeAnyway(null)).toBe(false);
  });
  it("spells running, dismissed and overridden", () => {
    expect(reviewLabel(review({ verdict: "running", findings: [] }))).toBe("review running");
    const r = review({ verdict: "clean", findings: [finding({ blocking: false, dismissed: { by: "tui", at: 1 } })] });
    expect(reviewLabel(r)).toBe("review clean (1 dismissed)");
    expect(reviewLabel(review({ merge_anyway: { by: "floris", at: 1 } }))).toBe("review overridden by floris");
  });
});
