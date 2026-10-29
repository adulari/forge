// The system-wide half of the Quick Composer shortcut. `shell.quickComposer` is one binding with
// two deliveries: the in-window hotkey (useHotkeys.web) and, on the desktop app, an OS-level
// registration made here so the chord also works while another application has focus. The shell
// answers a registration with a typed error; it is kept here for the shortcuts row to show.
import { useSyncExternalStore } from "react";

import { isTauri } from "../platform";
import { chordErrorStatus, type GlobalChordStatus } from "./globalChordCore";

let status: GlobalChordStatus = { state: "idle" };
const listeners = new Set<() => void>();

function setStatus(next: GlobalChordStatus): void {
  status = next;
  for (const listener of listeners) listener();
}

export function getGlobalChordStatus(): GlobalChordStatus {
  return status;
}

export function useGlobalChordStatus(): GlobalChordStatus {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getGlobalChordStatus,
    getGlobalChordStatus,
  );
}

let queue: Promise<void> = Promise.resolve();

/** Registers `accelerator` system-wide, replacing the previous chord. Never rejects, so a taken
 * chord cannot roll back the user's saved binding. */
export function syncGlobalQuickComposer(accelerator: string): Promise<void> {
  if (!isTauri) return Promise.resolve();
  queue = queue.then(async () => {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("quick_composer_register", { accelerator });
      setStatus({ state: "registered", accelerator });
    } catch (error) {
      setStatus(chordErrorStatus(accelerator, error));
    }
  });
  return queue;
}
