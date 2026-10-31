import { describe, expect, it, vi } from "vitest";

vi.mock("../../lib/platform", () => ({ isWeb: false }));

import { parsePayload } from "./sessionDrag";

describe("drag payloads", () => {
  it("carries a pane by its stable slot id", () => {
    expect(parsePayload(JSON.stringify({ kind: "pane", slot: 3 }))).toEqual({ kind: "pane", slot: 3 });
  });

  it("rejects the old positional pane payload and junk", () => {
    expect(parsePayload(JSON.stringify({ kind: "pane", index: 1 }))).toBeNull();
    expect(parsePayload("not json")).toBeNull();
    expect(parsePayload(undefined)).toBeNull();
  });

  it("still parses sessions and tabs", () => {
    expect(parsePayload(JSON.stringify({ kind: "session", id: "s" }))).toEqual({ kind: "session", id: "s" });
    expect(parsePayload(JSON.stringify({ kind: "tab", index: 2 }))).toEqual({ kind: "tab", index: 2 });
  });
});
