import { describe, expect, it } from "vitest";

import type { DiagnosticsResponse } from "./api";
import { assessCompatibility, buildSupportSummary } from "./diagnostics";

const diagnostics: DiagnosticsResponse = {
  checked_at: 1_785_500_000,
  host: {
    hostname: "secret-hostname",
    version: "1.4.0",
    protocol: 9,
    pid: 42,
    process_uptime_secs: 3600,
    os: "linux",
    arch: "x86_64",
  },
  resources: {
    process_memory_bytes: 1024,
    process_virtual_memory_bytes: 2048,
    system_total_memory_bytes: 8192,
    system_available_memory_bytes: 4096,
    cpu_count: 8,
    load_average_one: 0.25,
    load_average_five: 0.5,
    load_average_fifteen: 0.75,
  },
  runtime: {
    sessions: 3,
    busy_sessions: 1,
    waiting_sessions: 1,
    terminals: 2,
    terminal_clients: 1,
    web_push_ready: true,
    native_push_ready: false,
  },
  checks: [
    {
      id: "git",
      status: "ok",
      label: "attacker supplied",
      detail: "/private/workspace and token=abc",
      fix: "curl https://secret.example/token",
    },
    {
      id: "unknown",
      status: "warn",
      label: "connection URL",
      detail: "fany://secret/token",
      fix: null,
    },
  ],
};

describe("assessCompatibility", () => {
  it("requires a client update when the daemon protocol is newer", () => {
    expect(assessCompatibility(10, "2.0.0", 9, "1.0.0").status).toBe("client-outdated");
  });

  it("requires a daemon update when the daemon protocol is older", () => {
    expect(assessCompatibility(8, "0.9.0", 9, "1.0.0").status).toBe("daemon-outdated");
  });

  it("treats equal protocols with different versions as compatible skew", () => {
    expect(assessCompatibility(9, "1.4.0", 9, "1.5.0").status).toBe("version-skew");
  });

  it("reports old identity payloads as unknown", () => {
    expect(assessCompatibility(undefined, undefined, 9, "1.0.0").status).toBe("unknown");
  });
});

describe("buildSupportSummary", () => {
  it("copies only fixed aggregate fields and known check statuses", () => {
    const summary = buildSupportSummary(
      diagnostics,
      "1.5.0",
      9,
      {
        phase: "available",
        checkedAt: 123,
        availableVersion: "1.6.0",
        body: "release secret",
        message: "connection token",
      },
      "runtime-fingerprint",
    );

    expect(summary).toContain("daemon=1.4.0 protocol=9");
    expect(summary).toContain("native_runtime=runtime-fingerprint");
    expect(summary).toContain("Git=ok");
    expect(summary).toContain("desktop_update=available:1.6.0");
    expect(summary).not.toContain("secret-hostname");
    expect(summary).not.toContain("/private/workspace");
    expect(summary).not.toContain("token=abc");
    expect(summary).not.toContain("secret.example");
    expect(summary).not.toContain("release secret");
    expect(summary).not.toContain("connection token");
    expect(summary).not.toContain("fany://");
  });
});
