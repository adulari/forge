// One file's diff for the editor header and the diff view: a worktree session's whole-branch diff,
// or a shared tree's staged / working-tree diff. Both read the Review dock's own queries.
import { useGitDiff, useGitStatus, useSessionDiff } from "../../lib/queries";
import { type DiffBucket } from "../git/gitDiffEntries";

export function useFileDiff(sessionId: string, path: string, worktree: boolean) {
  const diff = useSessionDiff(sessionId, worktree);
  const status = useGitStatus(worktree ? null : sessionId);
  // A path can sit in both buckets; the working-tree side is what an agent just wrote.
  const staged = !!status.data && !status.data.unstaged.some((row) => row.path === path) && !status.data.untracked.some((row) => row.path === path) && status.data.staged.some((row) => row.path === path);
  const untracked = !!status.data && status.data.untracked.some((row) => row.path === path);
  const bucket: DiffBucket = staged ? "staged" : untracked ? "untracked" : "unstaged";
  const gitDiff = useGitDiff(sessionId, path, staged, { context: 3, enabled: !worktree && status.isSuccess });
  if (worktree) {
    return {
      file: diff.data?.files.find((file) => file.path === path) ?? null,
      loading: diff.isPending,
      error: diff.error,
      bucket,
    };
  }
  return {
    file: gitDiff.data?.files.find((file) => file.path === path) ?? gitDiff.data?.files[0] ?? null,
    loading: status.isPending || gitDiff.isPending,
    error: status.error ?? gitDiff.error,
    bucket,
  };
}

