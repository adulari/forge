// Pure half of the notification preferences: which alerts a person has asked for, and the rules
// that turn "a session finished / needs you / failed" into a toast, an OS notification, both or
// neither. Free of react-native imports so the routing rules are unit-testable.

export type AlertCategory = "done" | "attention" | "error";
export type AlertFilter = "all" | "unfocused";

export interface NotifyPrefs {
  /** A session finished its turn. */
  done: boolean;
  /** A session is waiting on a permission, a question or a plan. */
  attention: boolean;
  /** A session's turn ended in an error. */
  error: boolean;
  /** `all`: in-app toasts while Forge is in front plus OS notifications in the background.
   * `unfocused`: OS notifications only, and only while Forge is in the background. */
  filter: AlertFilter;
  sound: boolean;
  /** Leave the notification text out of anything an OS surface or the push relay can see. */
  hideText: boolean;
}

export const DEFAULT_NOTIFY_PREFS: NotifyPrefs = {
  done: true,
  attention: true,
  error: true,
  filter: "all",
  sound: false,
  hideText: false,
};

export const NOTIFY_PREFS_KEY = "forge.notifyPrefs.v1";

export function parseNotifyPrefs(raw: string | null): NotifyPrefs {
  if (!raw) return { ...DEFAULT_NOTIFY_PREFS };
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown> | null;
    const flag = (key: keyof NotifyPrefs, fallback: boolean) => (typeof parsed?.[key] === "boolean" ? (parsed[key] as boolean) : fallback);
    return {
      done: flag("done", DEFAULT_NOTIFY_PREFS.done),
      attention: flag("attention", DEFAULT_NOTIFY_PREFS.attention),
      error: flag("error", DEFAULT_NOTIFY_PREFS.error),
      filter: parsed?.filter === "unfocused" ? "unfocused" : "all",
      sound: flag("sound", DEFAULT_NOTIFY_PREFS.sound),
      hideText: flag("hideText", DEFAULT_NOTIFY_PREFS.hideText),
    };
  } catch {
    return { ...DEFAULT_NOTIFY_PREFS };
  }
}

export interface AlertRoute {
  /** Raise an in-app toast. */
  toast: boolean;
  /** Raise an OS notification. */
  os: boolean;
}

export interface AlertContext {
  category: AlertCategory;
  prefs: NotifyPrefs;
  /** The session the alert is about, when it has one. */
  sessionId: string | null;
  /** Sessions currently on screen: the routed one and every split pane. */
  visibleSessionIds: readonly string[];
  /** The window or tab is not in front. */
  backgrounded: boolean;
}

const SILENT: AlertRoute = { toast: false, os: false };

export function routeAlert(ctx: AlertContext): AlertRoute {
  if (!ctx.prefs[ctx.category]) return SILENT;
  if (ctx.backgrounded) return { toast: false, os: true };
  if (ctx.prefs.filter === "unfocused") return SILENT;
  if (ctx.sessionId && ctx.visibleSessionIds.includes(ctx.sessionId)) return SILENT;
  return { toast: true, os: false };
}

export const TITLE_MAX = 80;
export const BODY_MAX = 240;

/** Drop escape sequences and control characters, collapse whitespace, and cut to `max` characters
 * (counting code points) with an ellipsis. Alert text can echo a shell command or model output. */
export function sanitizeNotifyText(text: string, max: number): string {
  const stripped = text
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)/g, "")
    .replace(/\u001b./g, "")
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const chars = Array.from(stripped);
  if (chars.length <= max) return stripped;
  return `${chars.slice(0, max - 1).join("").trimEnd()}…`;
}

export function clipNotification(title: string, body?: string): { title: string; body: string | undefined } {
  return {
    title: sanitizeNotifyText(title, TITLE_MAX) || "Forge",
    body: body === undefined ? undefined : sanitizeNotifyText(body, BODY_MAX),
  };
}

/** A repeat of the same alert inside this window is dropped while the app is in front. */
export const FOREGROUND_DEDUPE_MS = 15_000;

export function isDuplicateAlert(lastShownAt: ReadonlyMap<string, number>, key: string, now: number): boolean {
  const last = lastShownAt.get(key);
  return last !== undefined && now - last < FOREGROUND_DEDUPE_MS;
}

export function rememberAlert(lastShownAt: Map<string, number>, key: string, now: number): void {
  lastShownAt.set(key, now);
  if (lastShownAt.size <= 64) return;
  for (const [k, at] of lastShownAt) if (now - at >= FOREGROUND_DEDUPE_MS) lastShownAt.delete(k);
}

export type AndroidChannel = "attention" | "status";

export const ANDROID_CHANNELS: Record<AndroidChannel, { name: string; description: string; importance: "high" | "low" }> = {
  attention: {
    name: "Needs attention",
    description: "A session is waiting on you, or a turn failed.",
    importance: "high",
  },
  status: {
    name: "Status",
    description: "Quiet updates, such as a session finishing.",
    importance: "low",
  },
};

/** Model-sent notices carry a priority; `high` is the "I need you" kind. */
export function channelForPriority(priority: string): AndroidChannel {
  return priority === "high" || priority === "urgent" ? "attention" : "status";
}

/** The category a model-sent notice belongs to, for the per-category switches. */
export function categoryForPriority(priority: string): AlertCategory {
  return channelForPriority(priority) === "attention" ? "attention" : "done";
}

export interface PwaPromptInput {
  /** Launched from the home screen rather than a browser tab. */
  standalone: boolean;
  isIOS: boolean;
  permission: "default" | "granted" | "denied" | "unsupported";
  dismissed: boolean;
}

/** iOS only lets a web app ask for permission from a tap, so an installed PWA shows its own
 * one-time prompt at launch while the answer is still open. */
export function pwaPromptDue(input: PwaPromptInput): boolean {
  return input.isIOS && input.standalone && input.permission === "default" && !input.dismissed;
}

export interface FleetRowState {
  id: string;
  title: string;
  busy: boolean;
  waiting: boolean;
  last_turn_outcome?: "success" | "failed" | null;
}

export interface FleetAlert {
  sessionId: string;
  title: string;
  category: AlertCategory;
}

/** What changed between two fleet snapshots: a session started waiting on you, or its turn ended
 * (failed, or finished). Waiting wins over finished when both flip in one snapshot. */
export function detectFleetAlerts(previous: ReadonlyMap<string, FleetRowState>, rows: readonly FleetRowState[]): FleetAlert[] {
  const alerts: FleetAlert[] = [];
  for (const row of rows) {
    const before = previous.get(row.id);
    if (!before) continue;
    const base = { sessionId: row.id, title: row.title.trim() || "A session" };
    if (row.waiting && !before.waiting) alerts.push({ ...base, category: "attention" });
    else if (!row.busy && before.busy && !row.waiting) {
      alerts.push({ ...base, category: row.last_turn_outcome === "failed" ? "error" : "done" });
    }
  }
  return alerts;
}

/**
 * Sessions on screen: the routed one plus every pane the grid actually renders, or just the
 * maximized pane. `rendered` is the pane grid's own count (panes are hidden from the end when the
 * window is narrow); null means no grid is mounted, so only the routed session is visible.
 */
export function visibleSessionIds(paneIds: readonly (string | null)[], maximized: number | null, rendered: number | null): string[] {
  const shown = rendered == null ? paneIds.slice(0, 1) : maximized != null ? [paneIds[maximized] ?? null] : paneIds.slice(0, rendered);
  return shown.filter((id): id is string => id != null);
}

export interface AlertCopy {
  title: string;
  body: string;
}

const VERB: Record<AlertCategory, string> = {
  done: "finished",
  attention: "needs you",
  error: "failed",
};

/** The toast line, and the OS notification title and body. With `hideText` the OS surface says
 * nothing about which session or what happened. */
export function alertCopy(alert: FleetAlert, hideText: boolean): AlertCopy & { toast: string } {
  const verb = VERB[alert.category];
  const toast = `${alert.title} ${verb}`;
  if (hideText) {
    return { toast, title: "Forge", body: alert.category === "done" ? "A session finished." : "A session needs you." };
  }
  return { toast, title: alert.title, body: `This session ${verb}.` };
}
