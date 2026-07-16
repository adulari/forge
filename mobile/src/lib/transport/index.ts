// Transport seam (ARCHITECTURE.md §6.3). `api.ts` and `ws.ts` call fetch/WebSocket ONLY
// through this module, never the platform global directly — this is the entire
// Tauri-specific data-path surface.
//
// On web/native this is a plain re-export of the platform globals — zero behavior change.
// T5.2's Tauri branch: on macOS/Linux a plain `http://` daemon (`--local` + VPN) can be
// blocked as mixed content by Tauri's WebView, so when `isTauri`:
//   - `tFetch` routes `http:` requests through `@tauri-apps/plugin-http` (executes in Rust,
//     immune to WebView mixed-content policy). `https:` requests still use the real fetch.
//   - `TWebSocket` tries the native `WebSocket` first; if it errors/closes before ever
//     opening a plain `ws:` connection, it falls back to a small WebSocket-compatible
//     adapter over `@tauri-apps/plugin-websocket`. `wss:` connections never need the
//     fallback (already a secure context) and go straight to native.
//
// Both tauri plugin packages are dynamic-`import()`ed and only ever reached behind an
// `isTauri` runtime check, so the web/native bundles never execute (or need) tauri code.
import type { Message as TauriWsMessage } from "@tauri-apps/plugin-websocket";

import { isTauri } from "../platform";

// ---------------------------------------------------------------------------
// fetch
// ---------------------------------------------------------------------------

function requestUrl(input: RequestInfo | URL): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  return input.url;
}

async function tauriAwareFetch(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<Response> {
  if (isTauri && requestUrl(input).startsWith("http:")) {
    const { fetch: tauriFetch } = await import("@tauri-apps/plugin-http");
    return tauriFetch(input as string | URL | Request, init);
  }
  return globalThis.fetch(input, init);
}

export const tFetch: typeof fetch = isTauri
  ? (tauriAwareFetch as typeof fetch)
  : globalThis.fetch.bind(globalThis);

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

type OpenHandler = ((ev: Event) => void) | null;
type MessageHandler = ((ev: MessageEvent) => void) | null;
type ErrorHandler = ((ev: Event) => void) | null;
type CloseHandler = ((ev: CloseEvent) => void) | null;

/**
 * WebSocket-compatible adapter used only when `isTauri`. Exposes just the surface
 * `ws.ts` actually uses (onopen/onmessage/onerror/onclose, send, close, readyState,
 * static OPEN) — the exported `TWebSocket` is cast to `typeof WebSocket` at the
 * boundary since that's the seam's contract, not because this class reimplements the
 * full DOM WebSocket interface.
 */
class TauriAwareWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  onopen: OpenHandler = null;
  onmessage: MessageHandler = null;
  onerror: ErrorHandler = null;
  onclose: CloseHandler = null;

  readyState: number = TauriAwareWebSocket.CONNECTING;

  private nativeWs: WebSocket | null = null;
  private pluginWs: import("@tauri-apps/plugin-websocket").default | null = null;
  private disposed = false;
  private fallbackStarted = false;
  private closeNotified = false;

  constructor(url: string) {
    if (url.startsWith("wss:")) {
      // Already a secure context — no mixed-content risk, native WebSocket is fine.
      this.attachNative(url, /* allowFallback */ false);
      return;
    }
    this.attachNative(url, /* allowFallback */ true);
  }

  private attachNative(url: string, allowFallback: boolean): void {
    const native = new WebSocket(url);
    this.nativeWs = native;
    let opened = false;

    native.onopen = () => {
      opened = true;
      this.readyState = TauriAwareWebSocket.OPEN;
      this.onopen?.(new Event("open"));
    };
    native.onmessage = (ev) => this.onmessage?.(ev);
    native.onerror = (ev) => {
      if (allowFallback && !opened) {
        // Likely a mixed-content block before the socket ever opened — fall back to
        // the Rust-side client, which is immune to WebView content policy.
        void this.fallbackToPlugin(url);
        return;
      }
      this.onerror?.(ev);
    };
    native.onclose = (ev) => {
      if (allowFallback && !opened && !this.disposed) {
        void this.fallbackToPlugin(url);
        return;
      }
      this.readyState = TauriAwareWebSocket.CLOSED;
      this.onclose?.(ev);
    };
  }

  private async fallbackToPlugin(url: string): Promise<void> {
    if (this.disposed || this.fallbackStarted) return;
    this.fallbackStarted = true;
    if (this.nativeWs) {
      this.nativeWs.onopen = null;
      this.nativeWs.onmessage = null;
      this.nativeWs.onerror = null;
      this.nativeWs.onclose = null;
      this.nativeWs.close();
    }
    this.nativeWs = null;

    try {
      const { default: TauriWebSocket } = await import("@tauri-apps/plugin-websocket");
      if (this.disposed) return;

      const sock = await TauriWebSocket.connect(url);
      if (this.disposed) {
        sock.disconnect().catch(() => {});
        return;
      }

      this.pluginWs = sock;
      this.readyState = TauriAwareWebSocket.OPEN;
      this.onopen?.(new Event("open"));

      sock.addListener((msg: TauriWsMessage) => {
        if (msg.type === "Text") {
          this.onmessage?.({ data: msg.data } as MessageEvent);
        } else if (msg.type === "Close") {
          this.emitClose({
            code: msg.data?.code ?? 1000,
            reason: msg.data?.reason ?? "",
          } as CloseEvent);
        }
        // Binary/Ping/Pong: the Forge WS protocol (ARCHITECTURE §3) is text-JSON only.
      });
    } catch (err) {
      this.terminateWithError(err, "tauri websocket connect failed");
    }
  }

  private emitClose(event: CloseEvent): void {
    if (this.closeNotified) return;
    this.closeNotified = true;
    this.readyState = TauriAwareWebSocket.CLOSED;
    this.onclose?.(event);
  }

  private terminateWithError(error: unknown, reason: string): void {
    if (this.disposed || this.closeNotified) return;
    this.disposed = true;
    const sock = this.pluginWs;
    this.pluginWs = null;
    if (sock) void sock.disconnect().catch(() => {});
    this.onerror?.(error as Event);
    this.emitClose({ code: 1006, reason } as CloseEvent);
  }

  send(data: string): void {
    if (this.pluginWs) {
      this.pluginWs.send(data).catch((err) => {
        this.terminateWithError(err, "tauri websocket send failed");
      });
    } else {
      this.nativeWs?.send(data);
    }
  }

  close(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.readyState = TauriAwareWebSocket.CLOSED;
    if (this.pluginWs) {
      this.pluginWs.disconnect().catch(() => {});
      this.pluginWs = null;
    }
    if (this.nativeWs) {
      this.nativeWs.close();
      this.nativeWs = null;
    }
  }
}

export const TWebSocket = (
  isTauri ? TauriAwareWebSocket : globalThis.WebSocket
) as unknown as typeof WebSocket;
