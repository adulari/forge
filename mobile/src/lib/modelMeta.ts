import type { ModelRow } from "./api";

/** How a model is paid for, as the Models screen labels it. */
export function modelAccess(model: Pick<ModelRow, "subscription" | "free">): "subscription" | "free" | "api" {
  return model.subscription ? "subscription" : model.free ? "free" : "api";
}

export function contextLabel(value: number | null | undefined): string | null {
  return value == null ? null : new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

/** One compact line for choosing a model: how it is paid for, its context window and coding
 * score, e.g. `subscription · 200K · code 62.1`. */
export function modelMetaLine(model: Pick<ModelRow, "subscription" | "free" | "context_window" | "benchmark_coding">): string {
  const code = model.benchmark_coding == null ? null : `code ${model.benchmark_coding.toFixed(1)}`;
  return [modelAccess(model), contextLabel(model.context_window), code].filter(Boolean).join(" · ");
}
