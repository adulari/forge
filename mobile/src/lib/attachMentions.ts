// The daemon stores an upload that rides a prompt as a leading `@<path>` mention on the user row
// (forge-cli `prepend_attach_mentions`), e.g. "@/x/.forge/uploads/s/1-notes.txt\n<typed text>".
// Requiring `.forge/uploads/` keeps this from ever matching a real `@mention` the user typed.
// A path with whitespace is braced (`@{/home/me/My Repo/.forge/uploads/…}`), as forge-cli's
// `mention_for` writes it.
const UPLOAD_MENTION = String.raw`(?:@\{[^}]*\.forge\/uploads\/[^}]+\}|@\S*\.forge\/uploads\/\S+)`;
export const ATTACH_MENTION_RE = new RegExp(String.raw`^((?:${UPLOAD_MENTION} ?)+)\n([\s\S]*)$`);
export const LEADING_ATTACH_MENTION_RE = new RegExp(String.raw`^((?:${UPLOAD_MENTION}\s*)+)`);

/** The upload paths in a run of leading mentions, braced or not. */
export function uploadMentionPaths(prefix: string): string[] {
  return [...prefix.matchAll(/@\{([^}]*)\}|@(\S+)/g)].map((m) => m[1] ?? m[2]);
}

/** The text the user typed: `content` without its leading upload mentions. */
export function stripAttachMentions(content: string): string {
  const m = content.match(ATTACH_MENTION_RE);
  return m ? m[2] : content;
}
