import type { PrReview, PrReviewFinding } from "../../lib/apiPr";

/** The row text, spelled like the daemon's queue row: `review: 2 confirmed, fixing`. */
export function reviewLabel(review: PrReview | null | undefined): string | null {
  if (!review) return null;
  if (review.merge_anyway) return `review overridden by ${review.merge_anyway.by}`;
  const confirmed = review.findings.filter((f) => f.blocking).length;
  const dismissed = review.findings.filter((f) => f.dismissed).length;
  const tail = dismissed > 0 && review.verdict !== "running" ? ` (${dismissed} dismissed)` : "";
  if (review.verdict === "running") return "review running";
  if (review.verdict === "clean") return `review clean${tail}`;
  if (review.verdict === "error") return "review failed, will retry";
  return (review.fix_sent ? `review: ${confirmed} confirmed, fixing` : `review: ${confirmed} confirmed`) + tail;
}

export function findingPlace(finding: PrReviewFinding): string {
  return finding.line != null ? `${finding.file}:${finding.line}` : finding.file;
}

/** The findings that hold the merge, worst first as the daemon ordered them. */
export function blockingFindings(review: PrReview | null | undefined): PrReviewFinding[] {
  return review ? review.findings.filter((f) => f.blocking) : [];
}

/** Findings that hold the merge, each with the 1-based number the daemon's dismiss action takes. */
export function numberedBlocking(review: PrReview | null | undefined): { n: number; finding: PrReviewFinding }[] {
  return review ? review.findings.flatMap((finding, i) => (finding.blocking ? [{ n: i + 1, finding }] : [])) : [];
}

/** "Merge anyway" applies while a review is holding, failing or still running the merge. */
export function canMergeAnyway(review: PrReview | null | undefined): boolean {
  return review != null && review.verdict !== "clean" && !review.merge_anyway;
}

export function withReview(detail: string | null, label: string | null): string | null {
  return [label, detail].filter(Boolean).join(" - ") || null;
}
