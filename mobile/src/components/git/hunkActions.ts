// Per-hunk Stage / Unstage / Revert for the review dock (POST /api/git/hunk). Pure: builds the
// single-hunk patch the daemon applies, decides which actions a card offers, and turns a failed
// request into what the card should do next.
import { ApiError, type GitDiffFile, type GitDiffHunk, type GitHunkAction } from "../../lib/api";
import { type DiffBucket } from "./gitDiffEntries";

export interface HunkActionSpec {
  action: GitHunkAction;
  label: string;
  /** Needs a confirmation before it runs. */
  destructive: boolean;
}

const STAGE: HunkActionSpec = { action: "stage", label: "Stage", destructive: false };
const UNSTAGE: HunkActionSpec = { action: "unstage", label: "Unstage", destructive: false };
const REVERT: HunkActionSpec = { action: "revert", label: "Revert", destructive: true };

/** The actions a hunk in `bucket` offers. A staged hunk can only be unstaged (revert acts on the
 * working tree); a new untracked file is staged or discarded as a whole, never reverted by hunk.
 * Nothing is offered where a patch cannot be built faithfully: binary files, renames, and files
 * whose diff the daemon cut short (a cut hunk is not a valid patch). */
export function hunkActionsFor(
  bucket: DiffBucket,
  file: Pick<GitDiffFile, "binary" | "kind" | "skipped_lines" | "orig_path">,
  readOnly: boolean,
): HunkActionSpec[] {
  if (readOnly || file.binary || file.kind === "renamed" || file.orig_path) return [];
  if (file.skipped_lines > 0) return [];
  if (bucket === "staged") return [UNSTAGE];
  if (bucket === "untracked") return [STAGE];
  return [STAGE, REVERT];
}

/** The `--- / +++ / @@` patch for one hunk of `file`, ready for `git apply`. */
export function buildHunkPatch(file: Pick<GitDiffFile, "path" | "kind">, hunk: GitDiffHunk): string {
  const from = file.kind === "created" ? "/dev/null" : `a/${file.path}`;
  const to = file.kind === "deleted" ? "/dev/null" : `b/${file.path}`;
  return `--- ${from}\n+++ ${to}\n${hunk.header}\n${hunk.lines.join("\n")}\n`;
}

export type HunkFailure =
  | { kind: "stale"; message: string }
  | { kind: "busy"; message: string }
  | { kind: "error"; message: string };

/** 409 `stale_patch`: the file changed since the diff was fetched, refetch rather than retry. */
export function classifyHunkError(error: unknown): HunkFailure {
  if (error instanceof ApiError && error.status === 409) {
    const code = (error.body as { code?: string } | undefined)?.code;
    if (code === "session_busy") return { kind: "busy", message: error.message };
    return { kind: "stale", message: "The file changed since this diff loaded. Refreshed." };
  }
  return { kind: "error", message: error instanceof Error ? error.message : "Could not apply the hunk." };
}

export function hunkActionDone(action: GitHunkAction): string {
  return action === "stage" ? "Hunk staged" : action === "unstage" ? "Hunk unstaged" : "Hunk reverted";
}
