import type { DeepPartial, JsonObject } from './types.js';

const blockedKeys = new Set(['__proto__', 'prototype', 'constructor']);
type SourceRecord = Record<string, unknown>;

function isRecord(value: unknown): value is SourceRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function ownEnumerableValue(source: SourceRecord, key: string): unknown {
  const descriptor = Object.getOwnPropertyDescriptor(source, key);
  return descriptor?.enumerable && 'value' in descriptor ? descriptor.value : undefined;
}

function defineValue(target: SourceRecord, key: string, value: unknown): void {
  Object.defineProperty(target, key, {
    configurable: true,
    enumerable: true,
    value,
    writable: true,
  });
}

function clone(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(clone);
  if (!isRecord(value)) return value;

  const result: SourceRecord = {};
  for (const key of Object.keys(value)) {
    if (blockedKeys.has(key)) continue;
    const item = ownEnumerableValue(value, key);
    defineValue(result, key, clone(item));
  }
  return result;
}

function mergeRecords(base: SourceRecord, override: SourceRecord): SourceRecord {
  const result = clone(base) as SourceRecord;
  for (const key of Object.keys(override)) {
    if (blockedKeys.has(key)) continue;
    const value = ownEnumerableValue(override, key);
    if (value === undefined) continue;
    const previous = ownEnumerableValue(result, key);
    defineValue(result, key, isRecord(previous) && isRecord(value)
      ? mergeRecords(previous, value)
      : clone(value));
  }
  return result;
}

export function mergeConfig<T extends JsonObject>(
  base: T,
  ...overrides: Array<DeepPartial<T> | undefined>
): T {
  let result = clone(base) as SourceRecord;
  for (const override of overrides) {
    if (override !== undefined && isRecord(override)) {
      result = mergeRecords(result, override);
    }
  }
  return result as T;
}
