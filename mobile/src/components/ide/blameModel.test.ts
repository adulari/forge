import { describe, expect, it } from "vitest";

import type { BlameLine } from "../../lib/api";
import { blameMap, isTraceable, runStarts, whenLabel, whySummary } from "./blameModel";

const line = (n: number, call: string, turn = 2): BlameLine => ({ line: n, session_id: "s", turn, model: "inferhub::cb/x", tool_call_id: call, prompt_summary: " fix it ", created_at: 100 });

describe("blame", () => {
  it("indexes by line and marks where a run starts", () => {
    const map = blameMap([line(3, "a"), line(4, "a"), line(5, "b"), line(9, "b")]);
    expect(map.get(4)?.tool_call_id).toBe("a");
    expect([...runStarts(map)].sort((x, y) => x - y)).toEqual([3, 5, 9]);
  });
  it("traces only new-side, non-deleted cells", () => {
    expect(isTraceable("new", "add")).toBe(true);
    expect(isTraceable("new", "context")).toBe(true);
    expect(isTraceable("old", "del")).toBe(false);
    expect(isTraceable("new", "del")).toBe(false);
  });
  it("summarises the turn and trims the prompt", () => {
    expect(whySummary(line(1, "a"))).toEqual({ title: "turn 2 · cb/x", detail: "fix it" });
  });
  it("words the age in seconds or milliseconds", () => {
    expect(whenLabel(null, 0)).toBeNull();
    expect(whenLabel(100, 130)).toBe("just now");
    expect(whenLabel(100, 100 + 600)).toBe("10 min ago");
    expect(whenLabel(1_700_000_000_000, 1_700_000_000 + 7300)).toBe("2 h ago");
  });
});
