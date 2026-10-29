import { describe, expect, it } from "vitest";

import type { ModelRow, ModelsResponse } from "./api";
import { catalogHasModel, catalogModels, deadPin, pickerSections, providerReadiness, pushRecent, runtimeLabel } from "./modelCatalog";

const model = (id: string, health: ModelRow["health"] = null): ModelRow => ({
  id,
  name: id.split("::")[1] ?? id,
  frontier: false,
  free: false,
  paid: false,
  subscription: true,
  estimated_cost_usd: 0,
  health,
});

const response = (providers: ModelsResponse["providers"]): ModelsResponse => ({
  catalog: "available",
  providers,
});

describe("catalogModels", () => {
  it("keeps every provider's models, bridge aliases included", () => {
    const data = response([
      { provider: "claude-cli", models: [model("claude-cli::fable"), model("claude-cli::sonnet")] },
      { provider: "groq", models: [model("groq::compound-mini")] },
    ]);

    expect(catalogModels(data).map(({ model: row }) => row.id)).toEqual([
      "claude-cli::fable",
      "claude-cli::sonnet",
      "groq::compound-mini",
    ]);
  });

  it("marks every alias of an excluded provider benched", () => {
    const excluded = { until_epoch: 42, reason: "invalid credentials" };
    const data = response([
      { provider: "claude-cli", models: [model("claude-cli::fable")], excluded },
    ]);

    expect(catalogModels(data)[0].model.health).toEqual(excluded);
    expect(providerReadiness(data.providers[0])).toEqual({ total: 1, ready: 0 });
  });

  it("leaves a model's own bench reason in place", () => {
    const own = { until_epoch: 7, reason: "rate limited" };
    const data = response([
      {
        provider: "claude-cli",
        models: [model("claude-cli::fable", own)],
        excluded: { until_epoch: 42, reason: "invalid credentials" },
      },
    ]);

    expect(catalogModels(data)[0].model.health).toEqual(own);
  });

  it("reports a routed model the catalog has never heard of", () => {
    const data = response([{ provider: "claude-cli", models: [model("claude-cli::sonnet")] }]);

    expect(catalogHasModel(data, "claude-cli::sonnet")).toBe(true);
    expect(catalogHasModel(data, "claude-cli::fable")).toBe(false);
    expect(catalogHasModel(undefined, "claude-cli::fable")).toBe(false);
  });
});

describe("picker sections", () => {
  const data = response([
    { provider: "claude-cli", models: [model("claude-cli::fable"), model("claude-cli::sonnet")] },
    { provider: "groq", models: [{ ...model("groq::m"), subscription: false }] },
  ]);
  const catalog = catalogModels(data);

  it("pins favourites then recents above the vendor groups", () => {
    const sections = pickerSections(catalog, ["groq::m"], ["claude-cli::sonnet", "groq::m"]);
    expect(sections.map((s) => s.key)).toEqual(["favorites", "recent", "vendor:claude-cli", "vendor:groq"]);
    expect(sections[1].rows.map((r) => r.model.id)).toEqual(["claude-cli::sonnet"]);
  });

  it("labels runtimes", () => {
    expect(runtimeLabel(catalog[0])).toBe("Subscription bridge");
    expect(runtimeLabel(catalog[2])).toBe("API");
  });

  it("keeps recents unique and capped", () => {
    expect(pushRecent(["a", "b"], "b")).toEqual(["b", "a"]);
    expect(pushRecent(["1", "2", "3", "4", "5"], "6")).toHaveLength(5);
  });
});

describe("deadPin", () => {
  const data = response([
    { provider: "claude-cli", models: [model("claude-cli::fable", { until_epoch: 1, reason: "rate limited" }), { ...model("claude-cli::sonnet"), benchmark_intelligence: 50 }] },
    { provider: "groq", models: [{ ...model("groq::m"), benchmark_intelligence: 90 }] },
  ]);

  it("is null for a healthy or empty pin", () => {
    expect(deadPin(data, "claude-cli::sonnet")).toBeNull();
    expect(deadPin(data, "")).toBeNull();
  });

  it("offers a same-provider replacement for a benched pin", () => {
    const dead = deadPin(data, "claude-cli::fable");
    expect(dead?.reason).toBe("rate limited");
    expect(dead?.replacement?.model.id).toBe("claude-cli::sonnet");
  });

  it("falls back across providers for a fake model", () => {
    const dead = deadPin(data, "nope::ghost");
    expect(dead?.replacement?.model.id).toBe("groq::m");
  });
});
