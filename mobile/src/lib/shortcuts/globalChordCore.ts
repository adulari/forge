// Pure half of globalChord.ts: the status a registration attempt leaves behind and the sentence
// the shortcuts row shows for it.
export type GlobalChordError = "invalid" | "unavailable" | "unsupported";

export type GlobalChordStatus =
  | { state: "idle" }
  | { state: "registered"; accelerator: string }
  | { state: "error"; accelerator: string; kind: GlobalChordError; message: string };

const ERROR_KINDS: readonly GlobalChordError[] = ["invalid", "unavailable", "unsupported"];

/** Turns whatever `invoke` rejected with into a status. The shell sends `{ kind, message }`; any
 * other rejection (plugin missing, IPC failure) is reported as the chord being unavailable. */
export function chordErrorStatus(accelerator: string, error: unknown): GlobalChordStatus {
  const candidate = error as { kind?: unknown; message?: unknown } | null;
  const kind = ERROR_KINDS.find((known) => known === candidate?.kind) ?? "unavailable";
  const message =
    typeof candidate?.message === "string" && candidate.message
      ? candidate.message
      : typeof error === "string" && error
        ? error
        : "The system refused this shortcut.";
  return { state: "error", accelerator, kind, message };
}

export function describeChordStatus(value: GlobalChordStatus): { tone: "ok" | "warn"; text: string } | null {
  if (value.state === "idle") return null;
  if (value.state === "registered") return { tone: "ok", text: "Also works from any app." };
  const lead =
    value.kind === "invalid"
      ? "This chord can't be a system-wide shortcut."
      : value.kind === "unsupported"
        ? "System-wide shortcut isn't available here."
        : "Couldn't claim this chord system-wide.";
  return { tone: "warn", text: `${lead} ${value.message}` };
}
