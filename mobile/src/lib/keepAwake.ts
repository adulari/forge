// Copy for the host's keep-awake setting (`[power] keep_awake`). The values are the daemon's own.

export const KEEP_AWAKE_MODES = ["off", "while_working", "while_remote", "always"] as const;
export type KeepAwakeMode = (typeof KEEP_AWAKE_MODES)[number];

export const KEEP_AWAKE_LABELS: Record<KeepAwakeMode, string> = {
  off: "Off",
  while_working: "Working",
  while_remote: "Remote",
  always: "Always",
};

const EXPLANATIONS: Record<KeepAwakeMode, string> = {
  off: "This computer sleeps whenever it normally would, even in the middle of a turn.",
  while_working: "Stays awake while a session is running a turn, and a short while after.",
  while_remote: "Stays awake while a turn is running or any app is connected to this host.",
  always: "Stays awake for as long as Forge is running on this host.",
};

/** Said under every mode: it stops sleep, not the lock screen. */
export const KEEP_AWAKE_LOCK_NOTE =
  "This only stops the computer sleeping. The screen can still lock, and closing a laptop lid may still suspend it.";

export function isKeepAwakeMode(value: string): value is KeepAwakeMode {
  return (KEEP_AWAKE_MODES as readonly string[]).includes(value);
}

export function keepAwakeExplanation(value: string): string {
  return isKeepAwakeMode(value) ? EXPLANATIONS[value] : "";
}
