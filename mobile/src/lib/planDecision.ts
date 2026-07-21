import type { QuestionOption } from "./ws";

export interface PlanDecisionBinding {
  build: string;
  cancel: string;
  promptSeq: number;
}

function normalized(value: string): string {
  return value.trim().replace(/\s+/g, " ").toLocaleLowerCase();
}

/** Bind plan actions only to the live question that explicitly names this plan. */
export function resolvePlanDecision(
  planTitle: string,
  question: string | null | undefined,
  options: QuestionOption[],
  promptSeq: number,
): PlanDecisionBinding | null {
  if (!question || !Number.isInteger(promptSeq) || promptSeq <= 0) return null;

  const normalizedQuestion = normalized(question);
  const normalizedTitle = normalized(planTitle);
  if (
    normalizedTitle.length === 0 ||
    !normalizedQuestion.includes("build this plan") ||
    !normalizedQuestion.includes(normalizedTitle)
  ) {
    return null;
  }

  const buildIndex = options.findIndex((option) => /\bbuild\b/i.test(option.label));
  const cancelIndex = options.findIndex((option) => /\bcancel\b/i.test(option.label));
  if (buildIndex < 0 || cancelIndex < 0 || buildIndex === cancelIndex) return null;

  return {
    build: String(buildIndex + 1),
    cancel: String(cancelIndex + 1),
    promptSeq,
  };
}
