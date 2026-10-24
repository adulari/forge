/**
 * The daemon forwards the terminal's permission line — "allow write_file (Write) [y/n]", and
 * since the call's subject rides along, "allow shell (Shell): cargo test [y/n]" — and the "[y/n]"
 * is a keyboard hint for the TUI. On a card with Allow/Deny buttons it reads as leaked terminal
 * text, so surfaces drop it, phrase the line as the question it is, and show the subject (the
 * command, the file) on its own rather than with a question mark glued to its end.
 */
export function permissionPromptParts(prompt: string): { question: string; subject: string | null } {
  const stripped = prompt.replace(/\s*\[[yn](\/[yn])?\]\s*$/i, "").trim();
  if (!stripped) return { question: prompt, subject: null };
  const named = /^(allow \S+ \([^)]*\)): (.+)$/is.exec(stripped);
  const head = named ? named[1] : stripped;
  const sentence = head.charAt(0).toUpperCase() + head.slice(1);
  return {
    question: /[?.!]$/.test(sentence) ? sentence : `${sentence}?`,
    subject: named ? named[2].trim() : null,
  };
}

export function displayPermissionPrompt(prompt: string): string {
  const { question, subject } = permissionPromptParts(prompt);
  return subject ? `${question} ${subject}` : question;
}
