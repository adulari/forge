import { describe, expect, it } from "vitest";

import type { PrQueueRow } from "../../lib/apiPr";
import { availableActions, pushesLabel, queueSummary, repoName, rowStatus, summaryLabel } from "./mergeQueueModel";

const row = (over: Partial<PrQueueRow> = {}): PrQueueRow => ({
  session_id: "s1",
  pr_number: 7,
  pr_url: "https://github.com/o/r/pull/7",
  position: 2,
  state: "queued",
  reason: null,
  pushes: 0,
  last_update: null,
  ...over,
});

describe("rowStatus", () => {
  it("labels every state", () => {
    expect(rowStatus(row({ state: "head", position: 1 }))).toMatchObject({ label: "Next to land", tone: "accent" });
    expect(rowStatus(row())).toMatchObject({ label: "Queued #2", tone: "neutral" });
    expect(rowStatus(row({ state: "queued", position: null })).label).toBe("Queued");
    expect(rowStatus(row({ state: "checks", last_update: "merged the base", reason: "waiting for checks" }))).toEqual({
      label: "Updated - in checks",
      tone: "accent",
      detail: "merged the base. waiting for checks",
    });
    expect(rowStatus(row({ state: "held", position: null }))).toMatchObject({ label: "Held", tone: "neutral" });
    expect(rowStatus(row({ state: "skipped", reason: "conflicts" }))).toEqual({ label: "Handed back", tone: "warn", detail: "conflicts" });
    expect(rowStatus(row({ state: "merged" }))).toMatchObject({ label: "Merged", tone: "success" });
    expect(rowStatus(row({ state: "blocked", reason: "ci failed" }))).toEqual({ label: "Blocked", tone: "danger", detail: "ci failed" });
  });

  it("treats the held flag as held whatever the state says", () => {
    expect(rowStatus(row({ state: "queued", held: true, position: null })).label).toBe("Held");
  });
});

describe("availableActions", () => {
  it("offers hold and front on a queued row behind another", () => {
    expect(availableActions(row())).toEqual(["hold", "front"]);
  });
  it("offers only hold on the head and on a first-position row", () => {
    expect(availableActions(row({ state: "head", position: 1 }))).toEqual(["hold"]);
    expect(availableActions(row({ state: "queued", position: 1 }))).toEqual(["hold"]);
  });
  it("offers hold and front while in checks", () => {
    expect(availableActions(row({ state: "checks" }))).toEqual(["hold", "front"]);
  });
  it("offers release and front on a held row", () => {
    expect(availableActions(row({ state: "held", position: null }))).toEqual(["release", "front"]);
    expect(availableActions(row({ held: true, position: null }))).toEqual(["release", "front"]);
  });
  it("offers retry only on a handed-back row", () => {
    expect(availableActions(row({ state: "skipped", position: null }))).toEqual(["retry"]);
  });
  it("offers nothing on finished rows", () => {
    expect(availableActions(row({ state: "merged", position: null }))).toEqual([]);
    expect(availableActions(row({ state: "blocked", position: null }))).toEqual([]);
  });
});

describe("queueSummary", () => {
  const queues = [
    { repo: "/a/.git", rows: [row({ state: "head", position: 1 }), row({ state: "checks" }), row({ state: "skipped", position: null })] },
    { repo: "/b/.git", rows: [row(), row({ held: true, state: "queued", position: null }), row({ state: "merged", position: null })] },
  ];
  it("counts waiting, held and handed-back rows across repos", () => {
    expect(queueSummary(queues)).toEqual({ waiting: 3, handedBack: 1, held: 1 });
    expect(summaryLabel(queueSummary(queues))).toBe("3 waiting, 1 held, 1 handed back");
  });
  it("says empty for no rows", () => {
    expect(summaryLabel(queueSummary([]))).toBe("Empty");
  });
});

describe("labels", () => {
  it("names a repo from its git dir", () => {
    expect(repoName("/home/me/code/forge/.git")).toBe("forge");
    expect(repoName("o/r")).toBe("r");
  });
  it("describes the queue's pushes", () => {
    expect(pushesLabel(row({ pushes: 1, last_update: "rebased" }))).toBe("1 push by the queue - rebased");
    expect(pushesLabel(row({ pushes: 2 }))).toBe("2 pushes by the queue");
    expect(pushesLabel(row())).toBeNull();
  });
});
