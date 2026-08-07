// Desktop auto-detect + offer-to-start (ARCHITECTURE.md §6 Tauri desktop shell). Bridges the
// three narrow Tauri commands in mobile/src-tauri/src/serve_discovery.rs — Tauri has no
// fs/shell plugin grant, so this is the only way the desktop app can see or start a local
// `forge serve` daemon. No-ops (resolve to the "nothing found" value) on every other platform,
// so callers never need their own `isTauri` guard.
import { isTauri } from "./platform";

export type ServeExposure = "local" | "lan" | "anywhere";

/** Mirrors `forge_cli::serve::ServeState` / the Rust `ServeState` in serve_discovery.rs. */
export interface DetectedServeState {
  pid: number;
  port: number;
  exposure: ServeExposure;
  base_url: string;
  token: string;
  started_at: number;
}

async function invokeTauri<T>(cmd: string): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd);
}

/**
 * Reads `<config_dir>/serve-state.json` via `detect_forge_serve`, which already validates the
 * pid is alive and the port is actually accepting connections — a `null`/thrown result both
 * mean "nothing usable found," collapsed here into `null` so callers don't need a try/catch.
 */
export async function detectForgeServe(): Promise<DetectedServeState | null> {
  if (!isTauri) return null;
  try {
    return await invokeTauri<DetectedServeState | null>("detect_forge_serve");
  } catch {
    return null;
  }
}

/** Whether a `forge` executable is on `PATH` — gates the "start a local server?" offer. */
export async function forgeBinaryAvailable(): Promise<boolean> {
  if (!isTauri) return false;
  try {
    return await invokeTauri<boolean>("forge_binary_available");
  } catch {
    return false;
  }
}

/**
 * Spawns `forge serve --local` detached and returns as soon as the process launches — NOT once
 * it's actually listening (that's what `pollForForgeServe` is for). Throws with a message
 * suitable for direct display if the spawn itself fails (e.g. the binary vanished from PATH
 * between the check and the click).
 */
export async function startForgeServe(): Promise<void> {
  await invokeTauri<void>("start_forge_serve");
}

/**
 * What the desktop shell should do on launch.
 *
 * `lan` is never auto-connected: it needs the self-signed certificate trusted first, so it stays
 * a deliberate choice rather than a connection that silently fails.
 */
export type DesktopConnectionPlan =
  | { kind: "connect"; state: DetectedServeState; started: boolean }
  | { kind: "confirm-lan"; state: DetectedServeState }
  | { kind: "manual"; reason: "no-binary" | "start-failed" | "start-timeout"; message?: string };

export interface DesktopConnectionProbes {
  detect: () => Promise<DetectedServeState | null>;
  binaryAvailable: () => Promise<boolean>;
  start: () => Promise<void>;
  poll: () => Promise<DetectedServeState | null>;
}

/**
 * Resolve the launch connection without asking the user to click through it.
 *
 * The desktop app used to detect a daemon and then wait behind a "Connect" button, and offer to
 * start one behind a second button — so a perfectly discoverable local daemon still needed two
 * clicks, and a missing one was a dead end with a URL to paste. Falling back should be automatic.
 *
 * Probes are injected so the ladder is testable without a Tauri shell.
 */
export async function resolveDesktopConnection(
  probes: DesktopConnectionProbes,
): Promise<DesktopConnectionPlan> {
  const found = await probes.detect();
  if (found) {
    return found.exposure === "lan"
      ? { kind: "confirm-lan", state: found }
      : { kind: "connect", state: found, started: false };
  }
  if (!(await probes.binaryAvailable())) {
    return { kind: "manual", reason: "no-binary" };
  }
  try {
    await probes.start();
  } catch (error) {
    return {
      kind: "manual",
      reason: "start-failed",
      message: error instanceof Error ? error.message : String(error),
    };
  }
  const started = await probes.poll();
  return started
    ? { kind: "connect", state: started, started: true }
    : { kind: "manual", reason: "start-timeout" };
}

/**
 * Polls `detectForgeServe` until it finds a live daemon or `timeoutMs` elapses. Used right
 * after `startForgeServe`, since the state file only appears after a successful bind — there's
 * no push signal, so polling is the only option.
 */
export async function pollForForgeServe(
  timeoutMs = 15_000,
  intervalMs = 500,
): Promise<DetectedServeState | null> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = await detectForgeServe();
    if (found) return found;
    if (Date.now() >= deadline) return null;
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}
