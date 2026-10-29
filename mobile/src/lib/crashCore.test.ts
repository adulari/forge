import { describe, expect, it } from "vitest";

import {
  RELOAD_LOOP_THRESHOLD,
  RELOAD_WINDOW_MS,
  buildCrashSummary,
  classifyCrash,
  errorMessage,
  isReloadLoop,
  nativeCrashLoopFromSearch,
  recentReloads,
  recordReload,
  type ReloadStorage,
} from "./crashCore";

function memoryStorage(): ReloadStorage {
  const map = new Map<string, string>();
  return { getItem: (key) => map.get(key) ?? null, setItem: (key, value) => void map.set(key, value) };
}

describe("classifyCrash", () => {
  it("calls anything before the first interactive frame a startup failure", () => {
    expect(classifyCrash("render", false)).toBe("startup");
    expect(classifyCrash("unhandled-rejection", false)).toBe("startup");
    expect(classifyCrash("script-error", false)).toBe("startup");
  });

  it("keeps the source once the app is up", () => {
    expect(classifyCrash("render", true)).toBe("render");
    expect(classifyCrash("unhandled-rejection", true)).toBe("unhandled-rejection");
    expect(classifyCrash("script-error", true)).toBe("script-error");
  });

  it("a native renderer loop is its own kind at any time", () => {
    expect(classifyCrash("native-renderer-loop", false)).toBe("renderer-loop");
    expect(classifyCrash("native-renderer-loop", true)).toBe("renderer-loop");
  });
});

describe("errorMessage", () => {
  it("reads errors, strings and message-bearing objects", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage("plain")).toBe("plain");
    expect(errorMessage({ message: "shaped" })).toBe("shaped");
    expect(errorMessage(42)).toBe("unknown error");
  });
});

describe("buildCrashSummary", () => {
  const env = { appVersion: "2.24.1", platform: "linux", startupComplete: true, reloadsRecently: 1 };

  it("scrubs urls, secrets and home paths from the message and the stack", () => {
    const summary = buildCrashSummary(
      {
        kind: "render",
        message: "fetch https://box.example.com/tok_abcdefghijklmnop failed for /home/alice/code/app.ts",
        stack: "Error: x\n    at open (/home/alice/code/forge/src/a.ts:10:2)\n    at run (http://127.0.0.1:7420/secret/b.js:1:1)",
      },
      env,
    );
    expect(summary).toContain("kind=render startup_complete=yes");
    expect(summary).toContain("client=2.24.1 platform=linux");
    expect(summary).not.toContain("alice");
    expect(summary).not.toContain("tok_abcdefghijklmnop");
    expect(summary).not.toContain("127.0.0.1");
  });

  it("says when there is no stack", () => {
    expect(buildCrashSummary({ kind: "startup", message: "x", stack: null }, { ...env, startupComplete: false })).toContain(
      "stack=unavailable",
    );
  });
});

describe("reload loop guard", () => {
  it("trips after the threshold within the window and forgets old reloads", () => {
    const storage = memoryStorage();
    const t0 = 1_000_000;
    for (let i = 0; i < RELOAD_LOOP_THRESHOLD - 1; i += 1) recordReload(storage, t0 + i * 1000);
    expect(isReloadLoop(storage, t0 + 5000)).toBe(false);
    recordReload(storage, t0 + 6000);
    expect(isReloadLoop(storage, t0 + 7000)).toBe(true);
    expect(recentReloads(storage, t0 + RELOAD_WINDOW_MS + 10_000)).toBe(0);
    expect(isReloadLoop(storage, t0 + RELOAD_WINDOW_MS + 10_000)).toBe(false);
  });

  it("survives missing, corrupt or throwing storage", () => {
    expect(isReloadLoop(null, 1)).toBe(false);
    const corrupt: ReloadStorage = { getItem: () => "{nope", setItem: () => undefined };
    expect(recentReloads(corrupt, 1)).toBe(0);
    const full: ReloadStorage = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(() => recordReload(full, 1)).not.toThrow();
  });
});

describe("nativeCrashLoopFromSearch", () => {
  it("reads the marker the desktop shell appends", () => {
    expect(nativeCrashLoopFromSearch("?native_crash=loop")).toBe(true);
    expect(nativeCrashLoopFromSearch("?a=1&native_crash=loop")).toBe(true);
    expect(nativeCrashLoopFromSearch("?native_crash=other")).toBe(false);
    expect(nativeCrashLoopFromSearch("")).toBe(false);
  });
});
