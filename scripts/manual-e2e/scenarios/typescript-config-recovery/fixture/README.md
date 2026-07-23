# Safe Config Merge

This zero-dependency ESM package exposes one public function:

```ts
mergeConfig<T extends JsonObject>(base: T, ...overrides: Array<DeepPartial<T> | undefined>): T
```

The public exports and their TypeScript names must remain unchanged. Internals may be split or
refactored.

Contract:

- Never mutate `base`, an override, or any nested object/array reachable from them.
- Recursively merge plain records. Arrays replace the previous array rather than merging by index.
- An `undefined` override value means “leave the previous value alone”; `null` is an explicit value.
- Ignore the dangerous keys `__proto__`, `prototype`, and `constructor` at every depth.
- Ignore inherited properties and return records whose prototypes cannot be polluted.
- The result must be detached: mutating its nested records or arrays cannot affect any input.
- Support any number of overrides, skipping an override that is itself `undefined`.

Restore the offline build and package entry point as part of the repair. `npm test` is the acceptance
command and must run without downloading packages.
