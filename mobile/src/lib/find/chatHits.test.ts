import { describe, expect, it } from "vitest";

import type { HistoryRow } from "../api";
import { isSearchableRow, mergeHits } from "./chatHits";

const row = (seq: number, content: string, extra: Partial<HistoryRow> = {}): HistoryRow =>
  ({ seq, role: "assistant", content, model: null, created_at: 0, visibility: "llm", ...extra }) as HistoryRow;
const text = (r: HistoryRow) => r.content;

describe("mergeHits", () => {
  it("unions loaded and remote hits, de-duplicated and oldest first", () => {
    const loaded = [row(9, "the Parser again"), row(7, "unrelated"), row(5, "parser")];
    const remote = [row(5, "parser"), row(2, "a parser note")];
    expect(mergeHits(loaded, remote, "parser", false, text)).toEqual([2, 5, 9]);
  });

  it("re-checks a host that ignored the query and sent an ordinary page", () => {
    const remote = [row(30, "nothing here"), row(29, "needle in here")];
    expect(mergeHits([], remote, "needle", false, text)).toEqual([29]);
  });

  it("honours case and the text the row shows", () => {
    expect(mergeHits([row(1, "Foo")], [], "foo", true, text)).toEqual([]);
    expect(mergeHits([row(1, "Foo")], [], "Foo", true, text)).toEqual([1]);
    expect(mergeHits([row(1, "<think>foo</think>bar")], [], "foo", false, () => "bar")).toEqual([]);
  });

  it("never searches tool, system or nudge rows", () => {
    expect(isSearchableRow(row(1, "x", { role: "tool" }))).toBe(false);
    expect(isSearchableRow(row(1, "x", { kind: "tool" }))).toBe(false);
    expect(isSearchableRow(row(1, "x", { role: "system" }))).toBe(false);
    expect(isSearchableRow(row(1, "x", { role: "user", kind: "nudge" }))).toBe(false);
    expect(isSearchableRow(row(1, "x", { role: "user" }))).toBe(true);
  });
});
