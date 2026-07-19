import type { RemoteSocket, RemoteTransport } from "./RemoteTransport";

/** Existing URL/token transport. It preserves direct HTTP, LAN, and user-managed tunnel behavior. */
export class DirectTransport implements RemoteTransport {
  readonly kind = "direct" as const;
  readonly authority = null;

  constructor(
    private readonly directFetch: typeof fetch,
    private readonly WebSocketImpl: typeof WebSocket,
  ) {}

  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
    return this.directFetch(input, init);
  }

  openWebSocket(url: string, protocols?: string | string[]): RemoteSocket {
    return protocols
      ? new this.WebSocketImpl(url, protocols)
      : new this.WebSocketImpl(url);
  }
}

