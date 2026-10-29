// "Enter sends as: Steer | Queue": what a prompt typed while a turn runs does. Steer hands it to
// the running turn at its next step (and queues it as a fallback); Queue only runs it after the
// turn. Cmd/Ctrl+Enter does the opposite of the setting. The setting is the daemon's
// `[chat] follow_up` (config key `chat.follow_up`), so the terminal and the app agree.
import type { ConfigField } from "./api";

export type FollowUpMode = "steer" | "queue";

export const FOLLOW_UP_KEY = "chat.follow_up";

export interface FollowUpSetting {
  mode: FollowUpMode;
  /** False for a daemon that predates `queue` / queue editing: it would drop those inputs. */
  supported: boolean;
}

export function followUpFromConfig(fields: readonly ConfigField[] | undefined): FollowUpSetting {
  const field = fields?.find((f) => f.key === FOLLOW_UP_KEY);
  if (!field) return { mode: "steer", supported: false };
  return { mode: field.value === "queue" ? "queue" : "steer", supported: true };
}

export function invertMode(mode: FollowUpMode): FollowUpMode {
  return mode === "steer" ? "queue" : "steer";
}

/** The mode one send uses: the setting, flipped by the alternate chord, steer on a daemon that
 * cannot queue-only. */
export function effectiveMode(setting: FollowUpSetting, invert: boolean): FollowUpMode {
  if (!setting.supported) return "steer";
  return invert ? invertMode(setting.mode) : setting.mode;
}
