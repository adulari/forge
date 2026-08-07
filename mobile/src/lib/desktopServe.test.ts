import { describe, expect, it, vi } from "vitest";

import {
  resolveDesktopConnection,
  type DesktopConnectionProbes,
  type DetectedServeState,
} from "./desktopServe";

vi.mock("./platform", () => ({ isTauri: true }));

const daemon = (overrides: Partial<DetectedServeState> = {}): DetectedServeState => ({
  pid: 1234,
  port: 7420,
  exposure: "local",
  base_url: "http://127.0.0.1:7420/token",
  token: "token",
  started_at: 0,
  ...overrides,
});

const probes = (overrides: Partial<DesktopConnectionProbes> = {}): DesktopConnectionProbes => ({
  detect: vi.fn(async () => null),
  binaryAvailable: vi.fn(async () => false),
  start: vi.fn(async () => {}),
  poll: vi.fn(async () => null),
  ...overrides,
});

describe("desktop launch connection", () => {
  it("connects to a running daemon without waiting for a click", async () => {
    const found = daemon();
    const plan = await resolveDesktopConnection(probes({ detect: vi.fn(async () => found) }));
    expect(plan).toEqual({ kind: "connect", state: found, started: false });
  });

  it("starts a daemon itself when none is running", async () => {
    const started = daemon({ pid: 4321 });
    const start = vi.fn(async () => {});
    const plan = await resolveDesktopConnection(
      probes({ binaryAvailable: vi.fn(async () => true), start, poll: vi.fn(async () => started) }),
    );
    expect(start).toHaveBeenCalledOnce();
    expect(plan).toEqual({ kind: "connect", state: started, started: true });
  });

  it("does not auto-connect LAN, which needs its certificate trusted first", async () => {
    const lan = daemon({ exposure: "lan" });
    const plan = await resolveDesktopConnection(probes({ detect: vi.fn(async () => lan) }));
    expect(plan).toEqual({ kind: "confirm-lan", state: lan });
  });

  it("falls back to manual entry when there is no forge binary to start", async () => {
    const start = vi.fn(async () => {});
    const plan = await resolveDesktopConnection(probes({ start }));
    expect(start).not.toHaveBeenCalled();
    expect(plan).toEqual({ kind: "manual", reason: "no-binary" });
  });

  it("reports a spawn failure instead of pretending it started", async () => {
    const plan = await resolveDesktopConnection(
      probes({
        binaryAvailable: vi.fn(async () => true),
        start: vi.fn(async () => {
          throw new Error("forge vanished from PATH");
        }),
      }),
    );
    expect(plan).toEqual({
      kind: "manual",
      reason: "start-failed",
      message: "forge vanished from PATH",
    });
  });

  it("reports a daemon that never came up", async () => {
    const plan = await resolveDesktopConnection(
      probes({ binaryAvailable: vi.fn(async () => true) }),
    );
    expect(plan).toEqual({ kind: "manual", reason: "start-timeout" });
  });
});
