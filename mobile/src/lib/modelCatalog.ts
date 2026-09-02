// One reading of `GET /api/models` for every surface that shows models (the Models & mesh screen,
// the session model picker, the provider readiness counts). Screens must not flatten the response
// themselves: the daemon reports two INDEPENDENT kinds of unusability and only one of them lives
// on the model row.
//
// A `health` row benches a single model id. A provider-wide exclusion (a rejected credential
// invalidates every alias at once) is keyed by PROVIDER, so it matches no model id — every screen
// that only looked at `model.health` drew those aliases green while `forge models` on the host
// printed them excluded. Folding the exclusion in here fixes all of them at once.
import type { ModelProvider, ModelRow, ModelsResponse } from "./api";

export interface CatalogModel {
  provider: string;
  model: ModelRow;
}

function withProviderExclusion(provider: ModelProvider): CatalogModel[] {
  return provider.models.map((model) => ({
    provider: provider.provider,
    model: model.health || !provider.excluded ? model : { ...model, health: provider.excluded },
  }));
}

/** Every model the daemon knows about, with provider-wide exclusions applied. */
export function catalogModels(data: ModelsResponse | undefined): CatalogModel[] {
  return (data?.providers ?? []).flatMap(withProviderExclusion);
}

/** How many of a provider's models the mesh can actually route to right now. */
export function providerReadiness(provider: ModelProvider): { total: number; ready: number } {
  const models = withProviderExclusion(provider);
  return { total: models.length, ready: models.filter(({ model }) => model.health == null).length };
}

/** Does the served catalog contain this pinned/routed id? A routing decision naming something the
 * list has never heard of means the host discovered models after this snapshot was taken. */
export function catalogHasModel(data: ModelsResponse | undefined, id: string): boolean {
  return (data?.providers ?? []).some((provider) => provider.models.some((model) => model.id === id));
}
