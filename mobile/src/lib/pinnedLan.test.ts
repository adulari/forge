import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const FP = `sha256:${"ab".repeat(32)}`;

describe("desktop certificate pinning", () => {
  beforeEach(() => {
    invoke.mockReset();
    vi.resetModules();
  });

  it("routes a pinned https origin through its loopback proxy, http and ws alike", async () => {
    invoke.mockResolvedValue(51234);
    const pinned = await import("./pinnedLan");
    await pinned.registerPin("https://192.168.1.5:7420/aabbccddeeff0011", FP);
    expect(invoke).toHaveBeenCalledWith("pinned_proxy", { host: "192.168.1.5", port: 7420, fingerprint: FP });
    expect(pinned.rewritePinned("https://192.168.1.5:7420/aabbccddeeff0011/api/sessions")).toBe(
      "http://127.0.0.1:51234/aabbccddeeff0011/api/sessions",
    );
    expect(pinned.rewritePinned("wss://192.168.1.5:7420/aabbccddeeff0011/ws")).toBe(
      "ws://127.0.0.1:51234/aabbccddeeff0011/ws",
    );
  });

  it("leaves unpinned origins alone", async () => {
    const pinned = await import("./pinnedLan");
    expect(pinned.rewritePinned("https://box.ts.net/aabbccddeeff0011/api")).toBe("https://box.ts.net/aabbccddeeff0011/api");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("reports a swapped certificate as a mismatch, and a dead host as unreachable", async () => {
    const pinned = await import("./pinnedLan");
    invoke.mockResolvedValueOnce("mismatch");
    expect(await pinned.probePin("https://10.0.0.2:7420/x", FP)).toBe("mismatch");
    invoke.mockRejectedValueOnce(new Error("timed out"));
    expect(await pinned.probePin("https://10.0.0.2:7420/x", FP)).toBe("unreachable");
  });

  it("re-pairing with a new fingerprint replaces the old proxy", async () => {
    invoke.mockResolvedValueOnce(1111).mockResolvedValueOnce(2222);
    const pinned = await import("./pinnedLan");
    await pinned.registerPin("https://10.0.0.2:7420/x", FP);
    await pinned.registerPin("https://10.0.0.2:7420/x", `sha256:${"cd".repeat(32)}`);
    expect(pinned.rewritePinned("https://10.0.0.2:7420/x/api")).toBe("http://127.0.0.1:2222/x/api");
  });
});
