// Pure helper, split out of DiffCard.tsx so it's importable from a plain vitest module (that
// file pulls in react-native).

export const HEAD_ELLIPSIS_MAX = 42;

/** Mono "head-ellipsis": keeps the tail of a long path, prefixed with an ellipsis — but the
 * basename is never sacrificed for it. A plain tail-keep (`…${path.slice(-N)}`) can still cut
 * the basename off entirely once a long directory segment (e.g. a worktree id) pushes it past
 * the character budget (`…e/worktrees/` for `.../.forge/worktrees/<uuid>/android-test.txt`,
 * losing the one thing this header exists to show). Reserve the basename first, then fill
 * whatever budget remains with the tail of the directory. */
export function headEllipsis(path: string, max: number = HEAD_ELLIPSIS_MAX): string {
  if (path.length <= max) return path;
  const lastSlash = path.lastIndexOf("/");
  if (lastSlash === -1) return `…${path.slice(-(max - 1))}`;
  const basename = path.slice(lastSlash + 1);
  const dir = path.slice(0, lastSlash);
  // An unusually long filename still can't fit the whole budget — fall back to the old
  // tail-keep for the basename itself rather than showing nothing useful at all.
  if (basename.length > max - 2) return `…${basename.slice(-(max - 1))}`;
  const dirBudget = max - basename.length - 2; // "…" + "/"
  const dirTail = dir.length > dirBudget ? dir.slice(-dirBudget) : dir;
  return `…${dirTail}/${basename}`;
}
