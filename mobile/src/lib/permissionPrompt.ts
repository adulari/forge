/**
 * The daemon forwards the terminal's permission line verbatim — "allow write_file (Write) [y/n]"
 * — and the "[y/n]" is a keyboard hint for the TUI. On a card with Allow/Deny buttons it reads as
 * leaked terminal text, so surfaces drop it and phrase the line as the question it is.
 */
export function displayPermissionPrompt(prompt: string): string {
  const stripped = prompt.replace(/\s*\[[yn](\/[yn])?\]\s*$/i, "").trim();
  if (!stripped) return prompt;
  const sentence = stripped.charAt(0).toUpperCase() + stripped.slice(1);
  return /[?.!]$/.test(sentence) ? sentence : `${sentence}?`;
}
