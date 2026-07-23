import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeConfig } from '@fixture/safe-config-merge';

test('recursively merges records, replaces arrays, and respects undefined/null', () => {
  const base = {
    server: { host: '127.0.0.1', port: 8080, tls: { enabled: false, ca: 'base' } },
    labels: ['base'],
    optional: 'keep',
    nullable: 'value',
  };
  const first = {
    server: { port: 9090, tls: { enabled: true } },
    labels: ['first', 'second'],
    optional: undefined,
    nullable: null,
  };
  const second = { server: { tls: { ca: 'override' } } };

  assert.deepEqual(mergeConfig(base, first, undefined, second), {
    server: { host: '127.0.0.1', port: 9090, tls: { enabled: true, ca: 'override' } },
    labels: ['first', 'second'],
    optional: 'keep',
    nullable: null,
  });
});

test('does not mutate or retain nested aliases to any input', () => {
  const base = { nested: { base: true }, array: [{ id: 1 }] };
  const override = { nested: { added: true }, array: [{ id: 2 }] };
  const beforeBase = structuredClone(base);
  const beforeOverride = structuredClone(override);
  const result = mergeConfig(base, override);

  result.nested.base = false;
  result.array[0].id = 99;
  assert.deepEqual(base, beforeBase);
  assert.deepEqual(override, beforeOverride);
});

test('ignores inherited and pollution keys at every depth', () => {
  const inherited = Object.create({ inherited: 'nope' });
  inherited.safe = { value: 1 };
  Object.defineProperty(inherited.safe, '__proto__', {
    value: { polluted: true }, enumerable: true, configurable: true,
  });
  inherited.safe.constructor = { prototype: { polluted: true } };
  inherited.safe.prototype = { polluted: true };

  const result = mergeConfig({ safe: { original: true } }, inherited);
  assert.deepEqual(result, { safe: { original: true, value: 1 } });
  assert.equal(result.inherited, undefined);
  assert.equal({}.polluted, undefined);
});

test('deeply detaches base even when there are no overrides', () => {
  const base = { one: { two: [{ three: 3 }] } };
  const result = mergeConfig(base);
  result.one.two[0].three = 4;
  assert.equal(base.one.two[0].three, 3);
});
