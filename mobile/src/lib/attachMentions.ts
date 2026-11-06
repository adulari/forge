// The daemon stores an upload that rides a prompt as a leading `@<path>` mention on the user row
// (forge-cli `prepend_attach_mentions`), e.g. "@/x/.forge/uploads/s/1-notes.txt\n<typed text>".
// Requiring `.forge/uploads/` keeps this from ever matching a real `@mention` the user typed.
export const ATTACH_MENTION_RE = /^((?:@\S*\.forge\/uploads\/\S+ ?)+)\n([\s\S]*)$/;

/** The text the user typed: `content` without its leading upload mentions. */
export function stripAttachMentions(content: string): string {
  const m = content.match(ATTACH_MENTION_RE);
  return m ? m[2] : content;
}
