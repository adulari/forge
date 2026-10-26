// Automatic issue reporting for the desktop and phone apps. Bugs (React render errors, unhandled
// rejections, Tauri panics) are scrubbed on-device by `reportingCore` and POSTed to the Forge
// relay, which files one GitHub issue per distinct problem. Modes: `auto` sends silently and shows
// a subtle "reported" mark, `ask` shows a card with a preview first, `off` sends nothing.
// Sending is always background work with a 5 s timeout and never blocks the UI.
import AsyncStorage from "@react-native-async-storage/async-storage";
import Constants from "expo-constants";
import { AppState, Platform } from "react-native";
import { useSyncExternalStore } from "react";

import { isTauri } from "./platform";
import {
  MAX_EVENTS,
  REPORTING_MODES,
  buildReport,
  classifyError,
  scrubText,
  slug,
  type ErrorKind,
  type Report,
  type ReportContext,
  type ReportSurface,
  type ReportingMode,
} from "./reportingCore";

export { REPORTING_MODES, NEVER_SENT, scrubText, classifyError } from "./reportingCore";
export type { ErrorKind, Report, ReportingMode } from "./reportingCore";

const MODE_KEY = "forge.reporting.mode";
const ID_KEY = "forge.reporting.installId";
const QUEUE_KEY = "forge.reporting.queue";
const SENT_KEY = "forge.reporting.sent";
const BACKOFF_KEY = "forge.reporting.backoffUntil";

export const DEFAULT_REPORTING_URL = "https://forge.adulari.dev/relay/v1/reports";
const MAX_QUEUE = 20;
const MAX_SENT_LOG = 20;
const SEND_TIMEOUT_MS = 5_000;
const SESSION_REPORT_CAP = 5;
const DEFAULT_BACKOFF_S = 300;
const MAX_BACKOFF_S = 3_600;

export interface SentEntry {
  at: number;
  report: Report;
}

export interface ReportingState {
  mode: ReportingMode;
  loaded: boolean;
  /** Ask mode: reports waiting for the user's decision. */
  pending: Report[];
  /** Last report that went out (auto or confirmed), newest first. */
  sent: SentEntry[];
  /** Epoch ms of the latest automatic send, for the transient "reported" mark. */
  lastAutoSentAt: number | null;
}

let state: ReportingState = { mode: "auto", loaded: false, pending: [], sent: [], lastAutoSentAt: null };
const listeners = new Set<() => void>();
const ring: { k: string; ms?: number; at: number }[] = [];
const seenThisSession = new Set<string>();
let sessionCount = 0;
let flushing: Promise<void> | null = null;
let envOverride: { url?: string; disabled?: boolean } = {};
let desktopVersion: string | null = null;
let installed = false;

function emit(patch: Partial<ReportingState>): void {
  state = { ...state, ...patch };
  for (const l of listeners) l();
}

export function subscribeReporting(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getReportingState(): ReportingState {
  return state;
}

export function useReporting(): ReportingState {
  return useSyncExternalStore(subscribeReporting, getReportingState, getReportingState);
}

/** Record a harness-style app event: a kind slug and optional duration, never content. */
export function recordEvent(kind: string, ms?: number): void {
  ring.push({ k: slug(kind), ...(ms !== undefined ? { ms } : {}), at: Date.now() });
  if (ring.length > MAX_EVENTS) ring.splice(0, ring.length - MAX_EVENTS);
}

function truthy(value: string | undefined): boolean {
  return value != null && ["1", "true", "yes", "on"].includes(value.trim().toLowerCase());
}

/** `FORGE_NO_REPORTS=1` (build-time `EXPO_PUBLIC_FORGE_NO_REPORTS`, or the desktop process env). */
function forcedOff(): boolean {
  return truthy(process.env.EXPO_PUBLIC_FORGE_NO_REPORTS) || envOverride.disabled === true;
}

export function reportingUrl(): string {
  const url = envOverride.url?.trim() || process.env.EXPO_PUBLIC_FORGE_REPORTING_URL?.trim();
  return url || DEFAULT_REPORTING_URL;
}

function devSuppressed(): boolean {
  const dev = typeof __DEV__ !== "undefined" && __DEV__;
  return dev && !envOverride.url && !process.env.EXPO_PUBLIC_FORGE_REPORTING_URL;
}

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!isTauri) return null;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<T>(cmd, args);
  } catch {
    return null;
  }
}

export function effectiveMode(): ReportingMode {
  return forcedOff() ? "off" : state.mode;
}

function parseMode(value: string | null): ReportingMode {
  return (REPORTING_MODES as readonly string[]).includes(value ?? "") ? (value as ReportingMode) : "auto";
}

export async function setReportingMode(mode: ReportingMode): Promise<void> {
  emit({ mode });
  if (mode === "off") {
    emit({ pending: [] });
    await AsyncStorage.removeItem(QUEUE_KEY);
  }
  await AsyncStorage.setItem(MODE_KEY, mode);
  // The Rust panic hook reads the same choice from a file in the app data dir.
  void tauriInvoke("set_reporting_mode", { mode });
  if (mode === "auto") void flushReports();
}

async function installId(): Promise<string> {
  const existing = await AsyncStorage.getItem(ID_KEY);
  if (existing && /^[0-9a-f]{32}$/.test(existing)) return existing;
  const bytes = new Uint8Array(16);
  const c = globalThis.crypto;
  if (c?.getRandomValues) c.getRandomValues(bytes);
  else for (let i = 0; i < 16; i += 1) bytes[i] = Math.floor(Math.random() * 256);
  const id = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  await AsyncStorage.setItem(ID_KEY, id);
  return id;
}

function surface(): ReportSurface {
  if (isTauri) return "desktop";
  if (Platform.OS === "ios") return "ios";
  if (Platform.OS === "android") return "android";
  return "web";
}

function hostOs(): { os: string; arch: string } {
  if (Platform.OS === "ios") return { os: "ios", arch: "aarch64" };
  if (Platform.OS === "android") return { os: "android", arch: "aarch64" };
  const ua = typeof navigator !== "undefined" ? `${navigator.platform ?? ""} ${navigator.userAgent ?? ""}` : "";
  const os = /Mac/i.test(ua) ? "macos" : /Win/i.test(ua) ? "windows" : /Linux|X11/i.test(ua) ? "linux" : "other";
  const arch = /arm64|aarch64/i.test(ua) ? "aarch64" : /x86_64|x64|Win64|amd64|Intel/i.test(ua) ? "x86_64" : "other";
  return { os, arch };
}

async function context(): Promise<ReportContext> {
  if (isTauri && !desktopVersion) {
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      desktopVersion = await getVersion();
    } catch {
      desktopVersion = null;
    }
  }
  const extra = Constants.expoConfig?.extra as { gitSha?: unknown } | undefined;
  const sha = String(process.env.EXPO_PUBLIC_GIT_SHA ?? extra?.gitSha ?? "");
  return {
    version: desktopVersion ?? Constants.expoConfig?.version ?? "unknown",
    git_sha: /^[0-9a-f]{7,12}$/.test(sha) ? sha : "unknown",
    surface: surface(),
    ...hostOs(),
    install_method: isTauri ? "installer" : Platform.OS === "web" ? "unknown" : "store",
    flags: [],
  };
}

async function readJson<T>(key: string, fallback: T): Promise<T> {
  try {
    const raw = await AsyncStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

// Queue read-modify-write cycles are serialized so a flush never overwrites a fresh enqueue.
let queueLock: Promise<unknown> = Promise.resolve();
function withQueue<T>(fn: () => Promise<T>): Promise<T> {
  const run = queueLock.then(fn, fn);
  queueLock = run.catch(() => undefined);
  return run;
}

function enqueue(report: Report): Promise<void> {
  return withQueue(async () => {
    const queue = await readJson<Report[]>(QUEUE_KEY, []);
    queue.push(report);
    await AsyncStorage.setItem(QUEUE_KEY, JSON.stringify(queue.slice(-MAX_QUEUE)));
  });
}

async function logSent(report: Report): Promise<void> {
  const at = Date.now();
  const sent = [{ at, report }, ...state.sent].slice(0, MAX_SENT_LOG);
  emit({ sent, ...(report.source === "auto" ? { lastAutoSentAt: at } : {}) });
  await AsyncStorage.setItem(SENT_KEY, JSON.stringify(sent));
}

async function postFetch(url: string, body: string, signal: AbortSignal): Promise<Response> {
  const init = { method: "POST", headers: { "Content-Type": "application/json" }, body, signal };
  if (isTauri) {
    const { fetch: tauriFetch } = await import("@tauri-apps/plugin-http");
    return tauriFetch(url, init);
  }
  return globalThis.fetch(url, init);
}

function retryAfterSeconds(response: Response): number {
  const raw = Number(response.headers.get("retry-after"));
  const seconds = Number.isFinite(raw) && raw > 0 ? raw : DEFAULT_BACKOFF_S;
  return Math.min(seconds, MAX_BACKOFF_S);
}

type SendOutcome = "sent" | "retry" | "drop";

async function sendOne(report: Report): Promise<SendOutcome> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), SEND_TIMEOUT_MS);
  try {
    const response = await postFetch(reportingUrl(), JSON.stringify(report), controller.signal);
    if (response.status >= 200 && response.status < 300) return "sent";
    if (response.status === 429 || response.status === 503) {
      const until = Date.now() + retryAfterSeconds(response) * 1000;
      await AsyncStorage.setItem(BACKOFF_KEY, String(until));
      return "retry";
    }
    return response.status >= 500 ? "retry" : "drop";
  } catch {
    await AsyncStorage.setItem(BACKOFF_KEY, String(Date.now() + 60_000));
    return "retry";
  } finally {
    clearTimeout(timer);
  }
}

/** Send whatever is queued, in the background. Safe to call at any time; never throws. */
export function flushReports(): Promise<void> {
  if (flushing) {
    // Something was queued while a pass was running: run once more when it ends.
    const current = flushing;
    flushAgain = current.then(() => runFlush());
    return flushAgain;
  }
  return runFlush();
}

let flushAgain: Promise<void> | null = null;

function runFlush(): Promise<void> {
  if (flushing) return flushing;
  flushing = (async () => {
    try {
      if (effectiveMode() === "off" || devSuppressed()) return;
      const until = Number(await AsyncStorage.getItem(BACKOFF_KEY)) || 0;
      if (Date.now() < until) return;
      const queue = await readJson<Report[]>(QUEUE_KEY, []);
      const finished: string[] = [];
      for (const report of queue) {
        const outcome = await sendOne(report);
        if (outcome === "retry") break;
        if (outcome === "sent") await logSent(report);
        finished.push(JSON.stringify(report));
      }
      // Re-read: something may have been queued while we were sending. Remove only what finished.
      await withQueue(async () => {
        const current = await readJson<Report[]>(QUEUE_KEY, []);
        for (const done of finished) {
          const at = current.findIndex((r) => JSON.stringify(r) === done);
          if (at >= 0) current.splice(at, 1);
        }
        await AsyncStorage.setItem(QUEUE_KEY, JSON.stringify(current));
      });
    } catch {
      // Reporting must never surface its own failure.
    }
  })().finally(() => {
    flushing = null;
  });
  return flushing;
}

/**
 * Report a caught error. Classifies it (expected conditions are dropped), scrubs it, then either
 * queues it for silent sending (auto) or parks it for the user's decision (ask).
 */
export async function reportError(kind: ErrorKind, error: unknown, site: string): Promise<void> {
  try {
    const mode = effectiveMode();
    if (mode === "off" || !classifyError(kind, error).report) return;
    if (sessionCount >= SESSION_REPORT_CAP) return;
    const report = buildReport({
      source: "auto",
      kind,
      site,
      error,
      context: await context(),
      installId: await installId(),
      events: ring,
      now: Date.now(),
    });
    if (seenThisSession.has(report.fingerprint)) return;
    seenThisSession.add(report.fingerprint);
    sessionCount += 1;
    await submit(report);
  } catch {
    // Reporting must never throw into the code path that is already failing.
  }
}

async function submit(report: Report): Promise<void> {
  if (effectiveMode() === "ask") {
    emit({ pending: [...state.pending, report].slice(-MAX_QUEUE) });
    return;
  }
  await enqueue(report);
  void flushReports();
}

/** Ask mode: the user chose Send. */
export async function approvePending(fingerprint: string, always = false): Promise<void> {
  const report = state.pending.find((r) => r.fingerprint === fingerprint);
  emit({ pending: state.pending.filter((r) => r.fingerprint !== fingerprint) });
  if (always) await setReportingMode("auto");
  if (!report) return;
  await enqueue(report);
  // A one-off approval must go out even though the mode stays `ask`.
  void flushReports();
}

/** Ask mode: the user chose Don't send. */
export function dismissPending(fingerprint: string): void {
  emit({ pending: state.pending.filter((r) => r.fingerprint !== fingerprint) });
}

/** Build the exact report a manual "Report a problem" would send, for the confirm preview. */
export async function buildManualReport(note: string, lastError?: string): Promise<Report> {
  return buildReport({
    source: "manual",
    kind: "manual",
    site: "app.report_a_problem",
    context: await context(),
    installId: await installId(),
    events: ring,
    now: Date.now(),
    userNote: note,
    lastError,
  });
}

/** Send a manual report the user has already previewed and confirmed. Works in `ask` mode. */
export async function sendManualReport(report: Report): Promise<boolean> {
  if (effectiveMode() === "off" || devSuppressedForManual()) return false;
  const outcome = await sendOne(report);
  if (outcome === "sent") {
    await logSent(report);
    return true;
  }
  if (outcome === "retry") await enqueue(report);
  return outcome === "retry";
}

function devSuppressedForManual(): boolean {
  return devSuppressed();
}

/** Scrubbed text of the most recent classified-or-not error, for the optional manual attachment. */
let lastErrorText = "";
export function lastErrorPreview(): string {
  return lastErrorText;
}
function rememberLastError(error: unknown): void {
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error ?? "");
  lastErrorText = scrubText(message, 300);
}

/** Reports the Rust panic hook queued before the app last exited. */
async function drainRustQueue(): Promise<void> {
  const files = await tauriInvoke<string[]>("take_queued_reports");
  if (!files?.length) return;
  const id = await installId();
  const ctx = await context();
  for (const raw of files.slice(0, MAX_QUEUE)) {
    try {
      const parsed = JSON.parse(raw) as Partial<Report>;
      const report = buildReport({
        source: "auto",
        kind: "tauri_panic",
        site: typeof parsed.site === "string" ? parsed.site : "tauri.panic",
        error: { message: String(parsed.message ?? ""), stack: undefined },
        context: ctx,
        installId: id,
        events: [],
        now: (Number(parsed.occurred_at) || Math.floor(Date.now() / 1000)) * 1000,
      });
      report.location = scrubText(String(parsed.location ?? ""), 160);
      report.fingerprint = typeof parsed.fingerprint === "string" && /^[0-9a-f]{16}$/.test(parsed.fingerprint)
        ? parsed.fingerprint
        : report.fingerprint;
      await submit(report);
    } catch {
      // A corrupt queue file is not worth a report.
    }
  }
}

/** Install global handlers and load persisted state. Idempotent; returns a disposer. */
export function installReporting(): () => void {
  if (installed) return () => undefined;
  installed = true;
  recordEvent("app_start");
  const cleanups: (() => void)[] = [];

  const gt = globalThis as unknown as {
    ErrorUtils?: {
      getGlobalHandler: () => (e: unknown, fatal?: boolean) => void;
      setGlobalHandler: (h: (e: unknown, fatal?: boolean) => void) => void;
    };
    addEventListener?: (t: string, h: (e: never) => void) => void;
    removeEventListener?: (t: string, h: (e: never) => void) => void;
  };
  if (gt.ErrorUtils && Platform.OS !== "web") {
    const previous = gt.ErrorUtils.getGlobalHandler();
    gt.ErrorUtils.setGlobalHandler((error, fatal) => {
      rememberLastError(error);
      void reportError("js_error", error, "app.global_handler");
      previous(error, fatal);
    });
    cleanups.push(() => gt.ErrorUtils?.setGlobalHandler(previous));
  }
  if (Platform.OS === "web" && gt.addEventListener) {
    const onError = (e: { error?: unknown; message?: string }) => {
      const error = e.error ?? e.message;
      rememberLastError(error);
      void reportError("js_error", error, "window.error");
    };
    const onRejection = (e: { reason?: unknown }) => {
      rememberLastError(e.reason);
      void reportError("js_unhandled_rejection", e.reason, "window.unhandledrejection");
    };
    gt.addEventListener("error", onError as never);
    gt.addEventListener("unhandledrejection", onRejection as never);
    cleanups.push(() => {
      gt.removeEventListener?.("error", onError as never);
      gt.removeEventListener?.("unhandledrejection", onRejection as never);
    });
  }
  const sub = AppState.addEventListener("change", (s) => {
    recordEvent(`app_${s}`);
    if (s === "active") void flushReports();
  });
  cleanups.push(() => sub.remove());

  void (async () => {
    envOverride = (await tauriInvoke<{ url?: string; disabled?: boolean }>("reporting_env")) ?? {};
    const mode = parseMode(await AsyncStorage.getItem(MODE_KEY));
    const sent = await readJson<SentEntry[]>(SENT_KEY, []);
    emit({ mode, sent, loaded: true });
    // Re-assert the choice for the Rust panic hook (first launch has no file yet).
    void tauriInvoke("set_reporting_mode", { mode });
    await drainRustQueue();
    void flushReports();
  })();

  return () => {
    for (const c of cleanups) c();
    installed = false;
  };
}

/** Called by the React error boundary. */
export function reportBoundaryError(error: Error): void {
  rememberLastError(error);
  void reportError("react_boundary", error, "react.error_boundary");
}

/** Called when a Tauri command fails in a way that indicates a bug (not a user-facing refusal). */
export function reportTauriCommandFailure(command: string, error: unknown): void {
  rememberLastError(error);
  void reportError("tauri_command", error, `tauri.${slug(command, 60)}`);
}

/** Test seam: reset module state. */
export function __resetReportingForTests(): void {
  state = { mode: "auto", loaded: false, pending: [], sent: [], lastAutoSentAt: null };
  ring.length = 0;
  seenThisSession.clear();
  sessionCount = 0;
  flushing = null;
  flushAgain = null;
  envOverride = {};
  installed = false;
  lastErrorText = "";
}

/**
 * Dev/test hook so an off-screen harness can provoke each path:
 * `globalThis.__forgeReportTest("boundary" | "rejection" | "error")`.
 */
(globalThis as { __forgeReportTest?: unknown }).__forgeReportTest = (which: string) => {
  const error = new Error(`forge report test ${which}`);
  if (which === "boundary") reportBoundaryError(error);
  else if (which === "rejection") void Promise.reject(error);
  else void reportError("js_error", error, "app.report_test");
};
