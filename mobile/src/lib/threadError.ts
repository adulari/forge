import type { ThreadError } from "./ws";

const TITLES: Record<ThreadError["category"], string> = {
  rate_limit: "Rate limited",
  auth: "Sign-in problem",
  context: "Context too long",
  network: "Network error",
  provider: "Provider error",
  other: "Turn failed",
};

export function threadErrorTitle(error: ThreadError): string {
  return TITLES[error.category] ?? TITLES.other;
}

/** Identity of one failure, so dismissing it does not hide a different one. */
export function threadErrorKey(error: ThreadError): string {
  return `${error.category}|${error.summary}|${error.details.length}`;
}

/** Whether the details say anything the headline does not. */
export function hasMoreDetail(error: ThreadError): boolean {
  return error.details.trim() !== error.summary.trim();
}
