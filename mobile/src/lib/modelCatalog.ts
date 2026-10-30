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

export type RuntimeLabel = "Subscription bridge" | "OAuth" | "API";

/** How a model is reached: a CLI bridge rides a subscription login, other subscription providers
 * use an OAuth sign-in, everything else is a keyed API. */
export function runtimeLabel({ provider, model }: CatalogModel): RuntimeLabel {
  if (!model.subscription) return "API";
  return provider.endsWith("-cli") ? "Subscription bridge" : "OAuth";
}

export interface PickerSection {
  key: string;
  title: string;
  rows: CatalogModel[];
}

/** Picker layout: favourites, then recents, then one group per vendor. A model appears once in
 * the pinned sections and again under its vendor so the vendor list stays complete. */
export function pickerSections(
  catalog: CatalogModel[],
  favorites: readonly string[],
  recents: readonly string[],
): PickerSection[] {
  const byId = new Map(catalog.map((row) => [row.model.id, row]));
  const pick = (ids: readonly string[]) => ids.flatMap((id) => byId.get(id) ?? []);
  const favoriteRows = catalog.filter(({ model }) => favorites.includes(model.id));
  const recentRows = pick(recents).filter(({ model }) => !favorites.includes(model.id));
  const vendors = new Map<string, CatalogModel[]>();
  for (const row of catalog) vendors.set(row.provider, [...(vendors.get(row.provider) ?? []), row]);
  return [
    { key: "favorites", title: "Favourites", rows: favoriteRows },
    { key: "recent", title: "Recent", rows: recentRows },
    ...[...vendors].map(([provider, rows]) => ({ key: `vendor:${provider}`, title: provider, rows })),
  ].filter((section) => section.rows.length > 0);
}

export interface DeadPin {
  reason: string;
  replacement: CatalogModel | null;
}

const benchmark = ({ model }: CatalogModel) => model.benchmark_intelligence ?? -Infinity;

/** A pin the catalog cannot serve right now (benched, provider excluded, or unknown) with the
 * nearest healthy replacement: same provider first, then the same tier anywhere, best benchmark. */
export function deadPin(data: ModelsResponse | undefined, pin: string | null | undefined): DeadPin | null {
  if (!pin || !data || data.catalog !== "available") return null;
  const catalog = catalogModels(data);
  const current = catalog.find(({ model }) => model.id === pin);
  if (current && !current.model.health) return null;
  const healthy = catalog.filter(({ model }) => !model.health && model.id !== pin);
  const provider = pin.includes("::") ? pin.split("::")[0] : current?.provider;
  const sameProvider = healthy.filter((row) => row.provider === provider);
  const sameTier = healthy.filter((row) => current != null && row.model.tier === current.model.tier);
  const best = (rows: CatalogModel[]) => [...rows].sort((a, b) => benchmark(b) - benchmark(a))[0] ?? null;
  return {
    reason: current?.model.health?.reason ?? "this model is not in the host's catalog",
    replacement: best(sameProvider) ?? best(sameTier) ?? best(healthy),
  };
}

const RECENTS_KEY = "forge.recentModels";
const RECENTS_MAX = 5;

export function pushRecent(recents: readonly string[], id: string): string[] {
  return [id, ...recents.filter((entry) => entry !== id)].slice(0, RECENTS_MAX);
}

export function loadRecents(): string[] {
  try {
    const parsed: unknown = JSON.parse(globalThis.localStorage?.getItem(RECENTS_KEY) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === "string") : [];
  } catch {
    return [];
  }
}

export function saveRecents(recents: readonly string[]): void {
  try {
    globalThis.localStorage?.setItem(RECENTS_KEY, JSON.stringify(recents));
  } catch {
    // storage blocked: recents are a convenience only
  }
}
