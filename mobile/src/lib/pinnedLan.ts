// Desktop certificate pinning for `--lan` daemons (mobile/src-tauri/src/pinned.rs). The WebView
// rejects a self-signed certificate, so a server whose pairing link carried `fp=sha256:..` is
// reached through a loopback proxy that dials the daemon itself and accepts exactly that leaf.
// Off Tauri every function is a no-op; iOS and Android pinning needs a native module and is not
// wired here.
import { isTauri } from "./platform";

export type PinResult = "match" | "mismatch" | "unreachable";

const pins = new Map<string, string>();
const proxyPorts = new Map<string, number>();
const pending = new Map<string, Promise<void>>();

function originOf(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" && parsed.protocol !== "wss:") return null;
    return `https://${parsed.host}`;
  } catch {
    return null;
  }
}

async function invokeTauri<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

function hostPort(origin: string): { host: string; port: number } {
  const url = new URL(origin);
  return { host: url.hostname.replace(/^\[|\]$/g, ""), port: Number(url.port || 443) };
}

/** Remember the pin for an origin and start its loopback proxy. Safe to call repeatedly. */
export function registerPin(baseUrl: string, fingerprint: string | undefined): Promise<void> {
  const origin = originOf(baseUrl);
  if (!isTauri || !origin || !fingerprint) return Promise.resolve();
  if (pins.get(origin) !== fingerprint) {
    pins.set(origin, fingerprint);
    proxyPorts.delete(origin);
    pending.delete(origin);
  }
  const running = pending.get(origin);
  if (running) return running;
  const { host, port } = hostPort(origin);
  const started = invokeTauri<number>("pinned_proxy", { host, port, fingerprint })
    .then((local) => {
      proxyPorts.set(origin, local);
    })
    .catch(() => {
      pending.delete(origin);
    });
  pending.set(origin, started);
  return started;
}

/** Swap a pinned origin for its loopback proxy. Unpinned URLs come back unchanged. */
export function rewritePinned(url: string): string {
  const origin = originOf(url);
  const port = origin ? proxyPorts.get(origin) : undefined;
  if (!origin || port === undefined) return url;
  const scheme = url.startsWith("wss:") ? "ws:" : "http:";
  return `${scheme}//127.0.0.1:${port}${url.slice(url.indexOf("/", url.indexOf("//") + 2))}`;
}

export async function rewritePinnedReady(url: string): Promise<string> {
  const origin = originOf(url);
  if (origin && pins.has(origin)) await pending.get(origin);
  return rewritePinned(url);
}

export function isPinned(url: string): boolean {
  const origin = originOf(url);
  return origin !== null && pins.has(origin);
}

/** Dial the daemon directly and compare its leaf with the pin. */
export async function probePin(baseUrl: string, fingerprint: string): Promise<PinResult> {
  const origin = originOf(baseUrl);
  if (!isTauri || !origin) return "unreachable";
  const { host, port } = hostPort(origin);
  try {
    return (await invokeTauri<string>("pinned_probe", { host, port, fingerprint })) === "match" ? "match" : "mismatch";
  } catch {
    return "unreachable";
  }
}

export interface NearbyHost {
  id: string;
  name: string;
  host: string;
  port: number;
  fingerprint: string | null;
}

/** `_forge._tcp` daemons on the local network. Desktop only; elsewhere the list is empty. */
export async function findNearbyHosts(windowMs = 1500): Promise<NearbyHost[]> {
  if (!isTauri) return [];
  try {
    return await invokeTauri<NearbyHost[]>("nearby_hosts", { windowMs });
  } catch {
    return [];
  }
}

export function nearbyBaseUrl(host: NearbyHost): string {
  return `https://${host.host}:${host.port}`;
}
