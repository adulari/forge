import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  MANUAL_FINGERPRINT,
  buildReport,
  classifyError,
  fingerprintOf,
  scrubText,
  type ReportContext,
} from "./reportingCore";

const storage = vi.hoisted(() => new Map<string, string>());

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async (key: string) => storage.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: string) => {
      storage.set(key, value);
    }),
    removeItem: vi.fn(async (key: string) => {
      storage.delete(key);
    }),
  },
}));
vi.mock("expo-constants", () => ({ default: { expoConfig: { version: "2.8.5" } } }));
vi.mock("react-native", () => ({
  AppState: { currentState: "active", addEventListener: vi.fn(() => ({ remove: vi.fn() })) },
  Platform: { OS: "ios" },
}));
vi.mock("react", () => ({ useSyncExternalStore: vi.fn() }));
vi.mock("./platform", () => ({ isTauri: false }));

const ctx: ReportContext = {
  version: "2.8.5",
  git_sha: "unknown",
  surface: "ios",
  os: "ios",
  arch: "aarch64",
  flags: [],
};

describe("scrubText privacy", () => {
  it("removes home paths, other paths, keys, emails, hosts, urls and ips", () => {
    const raw =
      "ENOENT /home/alice/work/secret-project/src/app.tsx and C:\\Users\\Bob\\code\\x.ts; " +
      "key sk-abcdefghijklmnopqrstuv, ghp_abcdefghijklmnopqrstuvwx, AKIAABCDEFGHIJKLMNOP, " +
      "Authorization: Bearer abc123def456ghi789, api_key=hunter2hunter2, " +
      "mail alice@example.com host alice-laptop.local 192.168.1.20:7420 " +
      "https://user:pw@box.example.org:8443/path/to/thing?token=zzz " +
      "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijk " +
      "blob 0123456789abcdef0123456789abcdef0123";
    const out = scrubText(raw, 2000);
    for (const leak of [
      "alice",
      "Bob",
      "/home",
      "secret-project",
      "sk-abc",
      "ghp_abc",
      "AKIAABC",
      "abc123def456",
      "hunter2",
      "example.com",
      "alice-laptop",
      "192.168",
      "user:pw",
      "path/to/thing",
      "token=zzz",
      "eyJhbGci",
      "0123456789abcdef",
    ]) {
      expect(out, `leaked ${leak}: ${out}`).not.toContain(leak);
    }
    expect(out).toContain("app.tsx");
    expect(out).toContain("<url:box.example.org>");
  });

  it("maps the home dir to ~ and truncates", () => {
    expect(scrubText("at /Users/carol/Library/x.js")).not.toContain("carol");
    expect(scrubText("a".repeat(1000) + " b", 300).length).toBeLessThanOrEqual(300);
  });
});

describe("classification", () => {
  it("does not report expected conditions", () => {
    const expected: unknown[] = [
      Object.assign(new Error("aborted"), { name: "AbortError" }),
      new TypeError("Network request failed"),
      new TypeError("Failed to fetch"),
      Object.assign(new Error("nope"), { status: 401 }),
      Object.assign(new Error("slow down"), { status: 429 }),
      new Error("Request timed out"),
      new Error("user cancelled"),
      new Error("Invalid URL"),
      new Error("You are offline"),
    ];
    for (const e of expected) expect(classifyError("js_error", e).report).toBe(false);
  });

  it("reports real bugs and manual reports", () => {
    expect(classifyError("react_boundary", new TypeError("x is not a function")).report).toBe(true);
    expect(classifyError("js_unhandled_rejection", new Error("Cannot read properties of undefined")).report).toBe(true);
    expect(classifyError("manual", undefined).report).toBe(true);
  });
});

describe("fingerprint", () => {
  it("ignores message text and depends on kind, site and frames", () => {
    const stack = (n: number) => `Error: boom ${n}\n    at renderRow (app://x/bundle.js:${n}:5)\n    at Screen (app://x/bundle.js:9:1)`;
    const a = buildReport({ source: "auto", kind: "js_error", site: "s", error: Object.assign(new Error("user bob typed"), { stack: stack(1) }), context: ctx, installId: "0".repeat(32), events: [], now: 0 });
    const b = buildReport({ source: "auto", kind: "js_error", site: "s", error: Object.assign(new Error("different"), { stack: stack(77) }), context: ctx, installId: "0".repeat(32), events: [], now: 0 });
    expect(a.fingerprint).toMatch(/^[0-9a-f]{16}$/);
    expect(a.fingerprint).toBe(b.fingerprint);
    expect(fingerprintOf("js_error", "other", a.frames)).not.toBe(a.fingerprint);
    expect(a.frames[0]).toBe("renderRow");
  });
});

describe("buildReport", () => {
  it("manual reports use the zero fingerprint, scrub notes and keep events content-free", () => {
    const r = buildReport({
      source: "manual",
      kind: "manual",
      site: "app.report_a_problem",
      context: ctx,
      installId: "a".repeat(32),
      events: [{ k: "Turn Start!", ms: 12.4, at: 1000 }],
      now: 4000,
      userNote: "broke in /home/dave/x/y.ts, key sk-abcdefghijklmnopqrstuv",
      lastError: "Error at /home/dave/z/w.ts",
    });
    expect(r.fingerprint).toBe(MANUAL_FINGERPRINT);
    expect(JSON.stringify(r)).not.toMatch(/dave|sk-abc/);
    expect(r.events).toEqual([{ k: "turn_start_", ms: 12, ago: 3 }]);
  });
});

describe("sending", () => {
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal("__DEV__", false);
    process.env.EXPO_PUBLIC_FORGE_REPORTING_URL = "http://relay.test/v1/reports";
  });

  it("auto mode posts a scrubbed report; 503 backs off and keeps it queued", async () => {
    const mod = await import("./reporting");
    mod.__resetReportingForTests();
    const calls: string[] = [];
    let status = 503;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, init: { body: string }) => {
        calls.push(init.body);
        return { status, headers: { get: () => "120" } };
      }),
    );
    await mod.reportError("react_boundary", new Error("crash in /home/zelda/app.tsx"), "react.error_boundary");
    await mod.flushReports();
    expect(calls).toHaveLength(1);
    expect(calls[0]).not.toContain("zelda");
    expect(Number(storage.get("forge.reporting.backoffUntil"))).toBeGreaterThan(Date.now());
    expect(JSON.parse(storage.get("forge.reporting.queue") ?? "[]")).toHaveLength(1);

    storage.delete("forge.reporting.backoffUntil");
    status = 202;
    await mod.flushReports();
    expect(calls).toHaveLength(2);
    expect(JSON.parse(storage.get("forge.reporting.queue") ?? "[]")).toHaveLength(0);
    expect(mod.getReportingState().sent).toHaveLength(1);
  });

  it("off mode sends nothing and ask mode parks the report", async () => {
    const mod = await import("./reporting");
    mod.__resetReportingForTests();
    const fetchMock = vi.fn(async () => ({ status: 202, headers: { get: () => null } }));
    vi.stubGlobal("fetch", fetchMock);
    await mod.setReportingMode("off");
    await mod.reportError("react_boundary", new Error("bug one"), "a");
    await mod.flushReports();
    expect(fetchMock).not.toHaveBeenCalled();

    await mod.setReportingMode("ask");
    await mod.reportError("react_boundary", new Error("bug two"), "b");
    expect(fetchMock).not.toHaveBeenCalled();
    const [pending] = mod.getReportingState().pending;
    expect(pending).toBeDefined();
    await mod.approvePending(pending.fingerprint, true);
    expect(mod.getReportingState().mode).toBe("auto");
    await mod.flushReports();

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("queue is bounded", async () => {
    const mod = await import("./reporting");
    mod.__resetReportingForTests();
    vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("offline"); }));
    for (let i = 0; i < 5; i += 1) await mod.reportError("react_boundary", new Error(`bug ${i}`), `site${i}`);
    expect(JSON.parse(storage.get("forge.reporting.queue") ?? "[]").length).toBeLessThanOrEqual(20);
  });
});
