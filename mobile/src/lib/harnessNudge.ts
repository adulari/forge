// The daemon injects a small number of synthetic "keep going" messages into a session's own
// transcript as if the user typed them, to push a stalled turn forward (see forge-core/src/
// lib.rs — EMPTY_DIFF_NUDGE et al, and the other `Store::add_nudge_message` call sites). They
// cross the wire as an ordinary `role: "user"` history row — a thinking-mode/tool-calling
// provider's next request still needs to end on a legal user turn — but a daemon on protocol
// v10+ now also tags the row `kind: "nudge"` (server: `migration_0036` + `map_history_row`,
// mobile-side type: `HistoryRow.kind` in lib/api.ts), independent of the exact wording.
//
// `kind` is preferred when present. The exact-text match below is kept ONLY as a fallback for a
// pre-v10 daemon that has no `kind` field to send (or sends kind=undefined) — it is fragile: if
// the daemon ever rewords a nudge, or adds a new one, an old daemon stops being recognized until
// this list is updated too. See the mobile bug report (#20) for the full context.
const KNOWN_HARNESS_NUDGES: readonly string[] = [
  "You have not modified any files. Implement the fix now — do not just describe it.",
];

export function isHarnessNudge(role: string, content: string, kind?: string): boolean {
  // A v10+ daemon says so directly — trust it, no text matching needed.
  if (kind === "nudge") return true;
  // Anything else (no `kind` at all from a pre-v9 daemon, or a `kind` that classified this row
  // as something other than a nudge on a daemon too old to know about nudges) falls back to the
  // exact-text heuristic, exactly as before.
  return role === "user" && KNOWN_HARNESS_NUDGES.includes(content);
}
