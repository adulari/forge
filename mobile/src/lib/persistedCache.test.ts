import { describe, expect, it } from "vitest";
import type { PersistedClient } from "@tanstack/react-query-persist-client";

import { PERSISTED_HISTORY_SESSIONS, trimPersistedClient } from "./persistedCache";

const query = (key: unknown[], data: unknown, dataUpdatedAt = 0) =>
  ({ queryKey: key, queryHash: JSON.stringify(key), state: { data, dataUpdatedAt } }) as unknown as PersistedClient["clientState"]["queries"][number];

describe("trimPersistedClient", () => {
  it("keeps the first history page of the newest sessions and drops large refetchable entries", () => {
    const history = Array.from({ length: PERSISTED_HISTORY_SESSIONS + 2 }, (_, i) =>
      query(["history", "b", `s${i}`, "tools"], { pages: [[{ seq: 2 }], [{ seq: 1 }]], pageParams: [undefined, 2] }, i),
    );
    const client = {
      timestamp: 0,
      buster: "",
      clientState: { mutations: [], queries: [...history, query(["changelog", "b"], "x".repeat(1000)), query(["sessions", "b"], [])] },
    } as unknown as PersistedClient;
    const out = trimPersistedClient(client).clientState.queries;
    const kept = out.filter((q) => q.queryKey[0] === "history");
    expect(kept).toHaveLength(PERSISTED_HISTORY_SESSIONS);
    expect(kept.map((q) => q.queryKey[2])).not.toContain("s0");
    expect((kept[0].state.data as { pages: unknown[] }).pages).toHaveLength(1);
    expect(out.some((q) => q.queryKey[0] === "changelog")).toBe(false);
    expect(out.some((q) => q.queryKey[0] === "sessions")).toBe(true);
  });
});
