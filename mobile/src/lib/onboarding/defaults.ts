import type { StepId } from "./types";

/** Steps that work with their defaults, so "set up everything recommended" settles them without
 * asking (keyless web search, shared benchmark data, notification and theme defaults).
 * Mirrors `forge_onboarding::DEFAULT_ONLY_STEPS`. */
export const DEFAULT_ONLY_STEPS: StepId[] = ["web_search", "benchmarks", "preferences"];
