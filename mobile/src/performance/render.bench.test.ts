import { describe, it } from "vitest";

import { highlightTokens } from "../lib/highlightTokens";
import { groupByPhase } from "../components/workflow/format";
import type { SnapshotSubagent } from "../lib/ws";
import { parseReasoning } from "../lib/reasoning";

function row(index: number, phase: string): SnapshotSubagent {
  return {
    id: `agent-${index}`,
    agent: "general",
    task: "benchmark",
    model: "benchmark",
    phase,
    last: "",
    done: index % 2 === 0,
    ok: true,
    cost: index / 100,
  };
}

function measure(label: string, iterations: number, operation: () => void): void {
  for (let i = 0; i < Math.min(10, iterations); i++) operation();
  const start = performance.now();
  for (let i = 0; i < iterations; i++) operation();
  const elapsed = performance.now() - start;
  console.info(`[render-benchmark] ${label}: ${(elapsed / iterations).toFixed(4)} ms/op (${iterations} iterations, ${elapsed.toFixed(2)} ms total)`);
}

describe("headless render-path benchmarks", () => {
  it("measures workflow grouping for 10k rows and 100 phases", () => {
    const phases = Array.from({ length: 100 }, (_, index) => `phase-${index}`);
    const rows = Array.from({ length: 10_000 }, (_, index) => row(index, phases[index % phases.length]));
    measure("groupByPhase 10k rows / 100 phases", 20, () => {
      groupByPhase(rows, phases);
    });
  });

  it("measures reasoning parsing for a long streamed message", () => {
    const message = `<think>${"reasoning token ".repeat(20_000)}</think>${"answer token ".repeat(20_000)}`;
    measure("parseReasoning 560kB streamed message", 20, () => {
      parseReasoning(message);
    });
  });

  it("measures syntax tokenization for a large diff/code block", () => {
    const code = Array.from({ length: 10_000 }, (_, index) => `const value${index} = ${index}; // changed line ${index}\n`).join("");
    measure("highlightTokens 420kB TypeScript block", 10, () => {
      highlightTokens(code, "typescript");
    });
  });
});
