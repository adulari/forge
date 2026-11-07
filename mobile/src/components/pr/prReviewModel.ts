import type { PrReview, PrReviewFinding } from "../../lib/apiPr";

/** The row text, spelled like the daemon's queue row: `review: 2 confirmed, fixing`. */
export function reviewLabel(review: PrReview | null | undefined): string | null {
  if (!review) return null;
  const confirmed = review.findings.filter((f) => f.blocking).length;
  if (review.verdict === "clean") return "review clean";
  if (review.verdict === "error") return "review failed, will retry";
  return review.fix_sent ? `review: ${confirmed} confirmed, fixing` : `review: ${confirmed} confirmed`;
}

export function findingPlace(finding: PrReviewFinding): string {
  return finding.line != null ? `${finding.file}:${finding.line}` : finding.file;
}

/** The findings that hold the merge, worst first as the daemon ordered them. */
export function blockingFindings(review: PrReview | null | undefined): PrReviewFinding[] {
  return review ? review.findings.filter((f) => f.blocking) : [];
}

export function withReview(detail: string | null, label: string | null): string | null {
  return [label, detail].filter(Boolean).join(" - ") || null;
}
