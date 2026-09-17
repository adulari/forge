// The daemon injects a small number of fixed, synthetic "keep going" messages into a session's
// own transcript as if the user typed them, to push a stalled turn forward (see forge-core/src/
// lib.rs — EMPTY_DIFF_NUDGE et al). They cross the wire as an ordinary `role: "user"`,
// `visibility: "llm"` history row: `store.add_message(&self.id, seq, Role::User,
// EMPTY_DIFF_NUDGE, None)` carries no `kind`/tag that marks it as harness-injected rather than
// something the person actually typed — verified against forge-core/src/lib.rs directly. That
// means the ONLY signal the client has is matching the exact, well-known nudge text itself.
//
// This is a heuristic, not a protocol-level distinction, and it is fragile: if the daemon ever
// rewords a nudge, or adds a new one, this stops recognizing it until updated here too. A robust
// fix needs the daemon to carry a `kind`/tag on these rows — out of scope here (no Rust changes
// in this pass); see the mobile bug report (#20) for the full context.
const KNOWN_HARNESS_NUDGES: readonly string[] = [
  "You have not modified any files. Implement the fix now — do not just describe it.",
];

export function isHarnessNudge(role: string, content: string): boolean {
  return role === "user" && KNOWN_HARNESS_NUDGES.includes(content);
}
