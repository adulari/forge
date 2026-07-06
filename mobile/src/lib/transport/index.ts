// Transport seam (ARCHITECTURE.md §6.3). `api.ts` and `ws.ts` call fetch/WebSocket ONLY
// through this module, never the platform global directly — this is the entire
// Tauri-specific data-path surface.
//
// Today both are plain re-exports of the platform globals — zero behavior change on
// native/web. T5.2 adds the Tauri branch here: on macOS/Linux a plain `http://` daemon
// (`--local` + VPN) can be blocked as mixed content by Tauri's WebView, so when
// `isTauri && baseUrl.startsWith("http:")`, `tFetch` swaps to `@tauri-apps/plugin-http`'s
// fetch (executes in Rust, immune to WebView mixed-content policy) and `TWebSocket` swaps to
// a small WebSocket-compatible adapter over `@tauri-apps/plugin-websocket`.
export const tFetch: typeof fetch = globalThis.fetch.bind(globalThis);
export const TWebSocket: typeof WebSocket = globalThis.WebSocket;
