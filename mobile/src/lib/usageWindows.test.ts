import { describe, expect, it } from "vitest";

import type { UsageProvider, UsageQuota } from "./api";
import { buildProviderBlocks, priceLabel, quotaSummary } from "./usageWindows";

const usage = (provider: string, kind: UsageProvider["kind"], costUsd: number): UsageProvider => ({
  provider,
  kind,
  inputTokens: 1000,
  cachedInputTokens: 0,
  outputTokens: 100,
  costUsd,
});

const quota = (provider: string, kind: UsageQuota["kind"], windowKind: string, fraction: number | null): UsageQuota => ({
  provider,
  kind,
  windowKind,
  status: "ok",
  resetsAt: 2_000_000_000,
  fraction,
  updatedAt: 1_000,
});

describe("buildProviderBlocks", () => {
  it("lists every Claude window, marking the unobserved ones unknown", () => {
    const [claude] = buildProviderBlocks([usage("claude-cli", "bridge", 0)], [quota("claude-cli", "bridge", "weekly", 0.4)]);
    expect(claude.quotas.map((q) => [q.windowKind, q.seen])).toEqual([
      ["five_hour", false],
      ["weekly", true],
      ["weekly_fable", false],
    ]);
    expect(quotaSummary(claude.quotas[0], 2_000)).toBe("unknown · not seen yet");
    expect(quotaSummary(claude.quotas[1], 4_600)).toMatch(/^40% · resets .* · seen 1h ago$/);
  });

  it("never bills a subscription, even for rows stored with a notional cost", () => {
    const blocks = buildProviderBlocks([usage("codex-oauth", "oauth", 67.84), usage("inferhub", "api", 0.02)], []);
    expect(blocks.map((b) => [b.provider, priceLabel(b)])).toEqual([
      ["inferhub", "$0.02"],
      ["codex-oauth", "Included"],
    ]);
    expect(blocks[1].quotas.map((q) => q.windowKind)).toEqual(["five_hour", "weekly"]);
  });

  it("shows the shared Codex account once", () => {
    const rows = [quota("codex-oauth", "oauth", "weekly", 0.1), quota("codex-cli", "bridge", "weekly", 0.1)];
    expect(buildProviderBlocks([], rows).map((b) => b.provider)).toEqual(["codex-oauth"]);
    expect(buildProviderBlocks([usage("codex-cli", "bridge", 0)], rows).map((b) => b.provider)).toEqual(["codex-cli"]);
  });

  it("does not invent windows for metered providers", () => {
    const [api] = buildProviderBlocks([usage("inferhub", "api", 0)], []);
    expect(api.quotas).toEqual([]);
    expect(priceLabel(api)).toBe("No price");
  });
});
