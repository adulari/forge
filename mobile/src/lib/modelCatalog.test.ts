import { describe, expect, it } from "vitest";

import type { ModelRow, ModelsResponse } from "./api";
import { catalogHasModel, catalogModels, providerReadiness } from "./modelCatalog";

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
