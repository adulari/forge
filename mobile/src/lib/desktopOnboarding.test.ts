import { beforeEach, describe, expect, it, vi } from "vitest";

import { installForgeCli, installForgeService, localStatus } from "./desktopOnboarding";

type Handler = (event: { payload: unknown }) => void;

const invoke = vi.fn();
const unlisten = vi.fn();
let handler: Handler | undefined;
const listen = vi.fn(async (_name: string, cb: Handler) => {
  handler = cb;
  return unlisten;
});

vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: Handler) => listen(name, cb),
}));

beforeEach(() => {
  invoke.mockReset();
  unlisten.mockReset();
  listen.mockClear();
  handler = undefined;
});

describe("desktop onboarding bridge", () => {
  it("reads the local status, and collapses a failure to null", async () => {
    invoke.mockResolvedValueOnce({ forge_binary: null });
    expect(await localStatus()).toEqual({ forge_binary: null });
    expect(invoke).toHaveBeenCalledWith("onboarding_local_status");
    invoke.mockRejectedValueOnce(new Error("boom"));
    expect(await localStatus()).toBeNull();
  });

  it("forwards install progress and unsubscribes when the install finishes", async () => {
    const seen: string[] = [];
    invoke.mockImplementationOnce(async () => {
      handler?.({ payload: { phase: "download", fraction: 0.5, message: "x" } });
      return { path: "/x/forge", version: "1.0.0" };
    });
    const result = await installForgeCli((p) => seen.push(p.phase));
    expect(result.version).toBe("1.0.0");
    expect(seen).toEqual(["download"]);
    expect(listen).toHaveBeenCalledWith("onboarding://install-progress", expect.any(Function));
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("unsubscribes even when the command rejects", async () => {
    invoke.mockRejectedValueOnce("checksum mismatch");
    await expect(installForgeService(() => undefined)).rejects.toBe("checksum mismatch");
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("does not subscribe when no progress callback is given", async () => {
    invoke.mockResolvedValueOnce({ running: true, serve: null });
    await installForgeService();
    expect(listen).not.toHaveBeenCalled();
  });
});
