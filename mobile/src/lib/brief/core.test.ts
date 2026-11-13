import { describe, expect, it } from "vitest";

import {
  awaySince,
  AWAY_THRESHOLD_SECS,
  cardTitle,
  headline,
  lineDetail,
  lineState,
  MAX_CARD_LINES,
  MAX_WINDOW_SECS,
  parseBrief,
  visibleLines,
  worthShowing,
  type Brief,
  type BriefSession,
} from "./core";

function session(over: Partial<BriefSession> = {}): BriefSession {
  return {
    id: "s1",
    title: "Fix the flake",
    project: "forge",
    outcome: "finished",
    reason: null,
    waitingSince: null,
    lastActivity: 1000,
    summary: "done",
    spendUsd: 0,
    unlanded: null,
    ...over,
  };
}

function brief(over: Partial<Brief> = {}): Brief {
  return {
    since: 0,
    now: 6 * 3600,
    totals: { sessions: 1, finished: 1, failed: 0, waiting: 0, interrupted: 0, working: 0, changesReady: 0, spendUsd: 0 },
    sessions: [session()],
    ...over,
  };
}

describe("awaySince", () => {
  it("is null for a first launch, a short gap and a missing stamp", () => {
    expect(awaySince(null, 10_000)).toBeNull();
    expect(awaySince(0, 10_000)).toBeNull();
    expect(awaySince(10_000 - AWAY_THRESHOLD_SECS + 1, 10_000)).toBeNull();
  });

  it("is the last visit once the gap passes the threshold, and never reaches back past the cap", () => {
    expect(awaySince(10_000 - AWAY_THRESHOLD_SECS, 10_000)).toBe(10_000 - AWAY_THRESHOLD_SECS);
    const now = MAX_WINDOW_SECS * 3;
    expect(awaySince(5, now)).toBe(now - MAX_WINDOW_SECS);
  });
});

describe("parseBrief", () => {
  it("maps the daemon shape and drops rows it cannot read", () => {
    const parsed = parseBrief({
      since: 1,
      now: 2,
      totals: { sessions: 2, finished: 1, failed: 1, changes_ready: 1, spend_usd: 3.1 },
      sessions: [
        { id: "a", title: "T", project: "p", outcome: "failed", reason: "provider 503", last_activity: 5, summary: "x", spend_usd: 1, unlanded: { added: 2, removed: 1 } },
        { id: "b", outcome: "mystery" },
        "junk",
      ],
    });
    expect(parsed.sessions).toHaveLength(1);
    expect(parsed.sessions[0]).toMatchObject({ id: "a", reason: "provider 503", unlanded: { added: 2, removed: 1 } });
    expect(parsed.totals).toMatchObject({ changesReady: 1, spendUsd: 3.1, waiting: 0 });
  });

  it("rejects a body that is not a brief", () => {
    expect(() => parseBrief({ nope: true })).toThrow();
  });
});

describe("wording", () => {
  it("heads the card with the window and lists only buckets that exist", () => {
    const b = brief({
      totals: { sessions: 6, finished: 3, failed: 1, waiting: 1, interrupted: 0, working: 0, changesReady: 2, spendUsd: 3.1 },
    });
    expect(cardTitle(b)).toBe("While you were away (6h)");
    expect(headline(b)).toBe("3 finished, 1 failed, 1 waiting on you, 2 with changes ready to land, $3.10 spent");
  });

  it("says how long a session has been waiting, why one failed, and what is ready to land", () => {
    expect(lineState(session({ outcome: "waiting", waitingSince: 1000 }), 1000 + 40 * 60)).toBe("was waiting on you for 40 min");
    expect(lineState(session({ outcome: "failed", reason: "provider 503" }), 0)).toBe("failed: provider 503");
    expect(lineDetail(session({ unlanded: { added: 4, removed: 2 }, spendUsd: 1.5 }))).toBe("+4/-2 to land · $1.50");
    expect(lineDetail(session())).toBe("");
  });

  it("folds lines past the cap into a count", () => {
    const many = brief({ sessions: Array.from({ length: MAX_CARD_LINES + 3 }, (_, i) => session({ id: `s${i}` })) });
    const { lines, hidden } = visibleLines(many);
    expect(lines).toHaveLength(MAX_CARD_LINES);
    expect(hidden).toBe(3);
  });

  it("is only worth showing when something happened", () => {
    expect(worthShowing(undefined)).toBe(false);
    expect(worthShowing(brief({ sessions: [] }))).toBe(false);
    expect(worthShowing(brief())).toBe(true);
  });
});
