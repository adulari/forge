import type { DeepPartial, JsonObject } from './types.js';

// Deliberately incomplete legacy implementation. Preserve the public API, not this implementation.
export function mergeConfig<T extends JsonObject>(
  base: T,
  ...overrides: Array<DeepPartial<T> | undefined>
): T {
  return Object.assign({}, base, ...overrides.filter(Boolean)) as T;
}
