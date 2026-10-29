// What the crash screen needs to say and share, kept free of React and native modules so it can
// be tested directly and so the boundary above every provider can import it safely.
import { scrubText } from "./reportingCore";

export type CrashSource = "render" | "script-error" | "unhandled-rejection" | "native-renderer-loop";
export type CrashKind = "startup" | "render" | "script-error" | "unhandled-rejection" | "renderer-loop";

export interface CrashInfo {
  kind: CrashKind;
  message: string;
  stack: string | null;
}

/** Anything that fails before the app first became interactive is a startup failure, whatever threw. */
export function classifyCrash(source: CrashSource, startupComplete: boolean): CrashKind {
  if (source === "native-renderer-loop") return "renderer-loop";
  if (!startupComplete) return "startup";
  return source;
}

export const CRASH_COPY: Record<CrashKind, { title: string; detail: string }> = {
  startup: {
    title: "Forge couldn't start",
    detail: "Something failed before the app finished loading.",
  },
  render: {
    title: "Forge hit an error drawing this screen",
    detail: "A screen threw while it was being drawn.",
  },
  "script-error": {
    title: "Forge stopped on an unexpected error",
    detail: "An uncaught error escaped the app.",
  },
  "unhandled-rejection": {
    title: "Forge stopped on an unhandled failure",
    detail: "A background task failed and nothing handled it.",
  },
  "renderer-loop": {
    title: "Forge's display process keeps crashing",
    detail: "The window's renderer crashed several times in a row, so Forge stopped reloading it.",
  },
};

export function errorMessage(value: unknown): string {
  if (value instanceof Error) return value.message || value.name;
  if (typeof value === "string") return value;
  if (value && typeof value === "object" && "message" in value) {
    const message = (value as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return "unknown error";
}

export function errorStack(value: unknown): string | null {
  return value instanceof Error && value.stack ? value.stack : null;
}

const STACK_FRAMES = 8;

export interface CrashEnvironment {
  appVersion?: string | null;
  platform: string;
  startupComplete: boolean;
  reloadsRecently: number;
}

/** Whitelisted, scrubbed text meant to be pasted into an issue. Messages go through the same
 * scrubber the error reporter uses, so tokens, URLs, paths and addresses never leave the device. */
export function buildCrashSummary(info: CrashInfo, env: CrashEnvironment): string {
  const frames = (info.stack ?? "")
    .split("\n")
    .slice(1, 1 + STACK_FRAMES)
    .map((line) => scrubText(line.trim(), 160))
    .filter(Boolean);
  return [
    "Forge crash summary",
    `kind=${info.kind} startup_complete=${env.startupComplete ? "yes" : "no"}`,
    `message=${scrubText(info.message, 300) || "none"}`,
    `client=${env.appVersion ?? "unknown"} platform=${env.platform}`,
    `reloads_last_2m=${env.reloadsRecently}`,
    `stack=${frames.length > 0 ? "" : "unavailable"}`,
    ...frames.map((frame) => `  ${frame}`),
  ].join("\n");
}

export const RELOAD_WINDOW_MS = 120_000;
export const RELOAD_LOOP_THRESHOLD = 3;
const RELOADS_KEY = "forge.crash.reloads";

export interface ReloadStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function readReloads(storage: ReloadStorage, now: number): number[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(RELOADS_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((at): at is number => typeof at === "number" && now - at < RELOAD_WINDOW_MS && at <= now);
  } catch {
    return [];
  }
}

export function recentReloads(storage: ReloadStorage | null, now: number): number {
  return storage ? readReloads(storage, now).length : 0;
}

/** Call right before reloading from the crash screen. */
export function recordReload(storage: ReloadStorage | null, now: number): void {
  if (!storage) return;
  try {
    storage.setItem(RELOADS_KEY, JSON.stringify([...readReloads(storage, now), now]));
  } catch {
    // Private mode or a full quota: the guard simply has no memory.
  }
}

/** Reloading has been tried repeatedly without the crash going away. */
export function isReloadLoop(storage: ReloadStorage | null, now: number): boolean {
  return recentReloads(storage, now) >= RELOAD_LOOP_THRESHOLD;
}

export const NATIVE_CRASH_PARAM = "native_crash";

export function nativeCrashLoopFromSearch(search: string): boolean {
  return new URLSearchParams(search).get(NATIVE_CRASH_PARAM) === "loop";
}

export const STARTUP_WATCHDOG_MS = 15_000;
