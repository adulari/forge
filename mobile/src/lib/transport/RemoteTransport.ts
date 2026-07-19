export interface RemoteSocket {
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: string | ArrayBufferLike | Blob | ArrayBufferView): void;
  close(code?: number, reason?: string): void;
}

/** Product-level transport for the unchanged Forge daemon HTTP/WebSocket protocol. */
export interface RemoteTransport {
  readonly kind: "direct" | "anywhere";
  readonly authority: string | null;
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
  openWebSocket(url: string, protocols?: string | string[]): RemoteSocket;
}

