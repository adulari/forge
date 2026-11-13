import { shortenPath } from "./pathDisplay";

/** A clean merge stages the branch's changes in the base repo without committing; this is the
 * toast text that tells the user where they are and what is left to do. */
export function mergeStagedMessage(base: string | undefined, branch: string): string {
  if (!base) return `Merged ${branch}. Changes are staged. Review & commit`;
  return `Changes are staged in ${shortenPath(base, 40)}. Review & commit`;
}
