import { describe, expect, it } from "vitest";

import { fleetStatuses } from "./fleetStatuses";

const server = [{ id: "s1", name: "laptop" }];
const rows = [{ id: "a", waiting: true }] as never;

describe("fleetStatuses", () => {
  it("is reachable while polls succeed", () => {
    const [status] = fleetStatuses(server, [{ data: rows, isPending: false, isSuccess: true, failureCount: 0 }], 1_000_000);
    expect(status.reachability).toBe("reachable");
    expect(status.waiting).toBe(1);
  });

  it("treats a single failed poll as a blip", () => {
    const [status] = fleetStatuses(
      server,
      [{ data: rows, isPending: false, isSuccess: false, failureCount: 1, isError: true, dataUpdatedAt: 995_000 }],
      1_000_000,
    );
    expect(status.reachability).toBe("reachable");
  });

  it("calls a host unreachable after a stretch of failed polls, keeping what it last reported", () => {
    const [status] = fleetStatuses(
      server,
      [{ data: rows, isPending: false, isSuccess: false, failureCount: 1, isError: true, dataUpdatedAt: 900_000 }],
      1_000_000,
    );
    expect(status.reachability).toBe("unreachable");
    expect(status.waiting).toBe(1);
  });

  it("never reports a pending host as unreachable", () => {
    const [status] = fleetStatuses(server, [{ isPending: true, isSuccess: false, failureCount: 0 }], 0);
    expect(status.reachability).toBe("pending");
  });
});
