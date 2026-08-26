import type { RemoteSocket, RemoteTransport } from "./RemoteTransport";
import { encodeRequestBody } from "./requestBody";

export type BridgeRoute =
  | "health"
  | "list_sessions"
  | "create_session"
  | "session_snapshot"
  | "session_history"
  | "session_input"
  | "archive_session"
  | "past_sessions"
  | "search_sessions"
  | "rename_session"
  | "delete_session"
  | "session_tree"
  | "fork_session"
  | "merge_session"
  | "discard_session"
  | "list_projects"
  | "browse_projects"
  | "upload"
  | "voice_transcribe"
  | "list_skills"
  | "list_models"
  | "read_config"
  | "update_config"
  | "list_hooks"
  | "list_plans"
  | "read_mcp"
  | "update_mcp"
  | "usage"
  | "diagnostics"
  | "answer"
  | "push_key"
  | "push_subscribe"
  | "push_unsubscribe"
  | "list_terminals"
  // Read-only git review. The mutating git endpoints (switch/stage/unstage/commit) are
  // deliberately absent: the host refuses them over the bridge, so listing them here would only
  // produce a request that comes back denied.
  | "git_status"
  | "git_branches"
  | "git_diff";

export interface AnywhereBridgeRequest {
  hostId: string;
  route: BridgeRoute;
  parameters: string[];
  method: string;
  headers: [string, string][];
  body: Uint8Array;
  /**
   * The caller's own deadline and cancellation. Without this the relay applied a flat cap of its
   * own to every route, so `transcribeAudio`'s 120s budget silently became 30s — and a voice clip
   * is transcribed on the host before it answers, so long recordings could never come back.
   */
  signal?: AbortSignal;
}

export interface AnywhereBridgeResponse {
  status: number;
  headers?: [string, string][];
  body: Uint8Array;
}

/** Encryption/ticket implementation supplied by the enrolled Anywhere account layer. */
export interface AnywhereRelay {
  request(request: AnywhereBridgeRequest): Promise<AnywhereBridgeResponse>;
  openSessionSocket(request: {
    hostId: string;
    sessionId: string;
    revision: number;
  }): RemoteSocket;
  openTerminalSocket?(request: {
    hostId: string;
    sessionId: string;
    terminalId: string;
    cols: number;
    rows: number;
    restart: boolean;
  }): RemoteSocket;
}

/** Encrypted managed transport. No path can fall through to an arbitrary relay proxy. */
export class AnywhereTransport implements RemoteTransport {
  readonly kind = "anywhere" as const;
  readonly authority: string;
  readonly baseUrl: string;

  constructor(
    readonly hostId: string,
    private readonly relay: AnywhereRelay,
  ) {
    this.authority = hostId;
    this.baseUrl = `fany://${hostId}`;
  }

  async fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
    const url = inputUrl(input);
    assertHost(url, this.hostId);
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const mapping = routeFor(url.pathname, method);
    // The body is encoded here rather than round-tripped through `new Request(...).arrayBuffer()`:
    // that route cannot serialise FormData on React Native (see requestBody.ts), which broke
    // voice upload and every attachment on phones paired through Anywhere.
    const encoded = await encodeRequestBody(init?.body);
    const headers = new Headers(init?.headers);
    // A multipart body only parses against the boundary we just generated, so the encoding the
    // body actually has wins over whatever content-type the caller guessed.
    if (encoded.contentType) headers.set("content-type", encoded.contentType);
    const response = await this.relay.request({
      hostId: this.hostId,
      route: mapping.route,
      parameters: url.search ? [...mapping.parameters, url.search] : mapping.parameters,
      method,
      headers: Array.from(headers.entries()),
      body: encoded.bytes,
      signal: init?.signal ?? undefined,
    });
    return new Response(response.body as unknown as BodyInit, {
      status: response.status,
      headers: response.headers,
    });
  }

  openWebSocket(urlValue: string): RemoteSocket {
    const url = new URL(urlValue);
    assertHost(url, this.hostId);
    if (url.pathname !== "/ws" && url.pathname !== "/ws/terminal") {
      throw new Error("Forge Anywhere only permits session and terminal streams");
    }
    const sessionId = url.searchParams.get("session");
    if (!sessionId) throw new Error("invalid Forge Anywhere stream session");
    if (url.pathname === "/ws") {
      const revision = Number(url.searchParams.get("rev") ?? "0");
      if (!Number.isSafeInteger(revision) || revision < 0) {
        throw new Error("invalid Forge Anywhere session stream parameters");
      }
      return this.relay.openSessionSocket({
        hostId: this.hostId,
        sessionId,
        revision,
      });
    }
    if (url.pathname === "/ws/terminal") {
      if (!this.relay.openTerminalSocket) {
        throw new Error("Forge Anywhere terminal streaming is unavailable");
      }
      const terminalId = url.searchParams.get("terminal") || "term-1";
      const cols = Number(url.searchParams.get("cols") ?? "80");
      const rows = Number(url.searchParams.get("rows") ?? "24");
      const restart = url.searchParams.get("restart") === "true";
      if (
        !/^[a-zA-Z0-9_-]{1,64}$/.test(terminalId) ||
        !Number.isSafeInteger(cols) ||
        !Number.isSafeInteger(rows) ||
        cols < 1 ||
        rows < 1 ||
        cols > 1_000 ||
        rows > 1_000
      ) {
        throw new Error("invalid Forge Anywhere terminal stream parameters");
      }
      return this.relay.openTerminalSocket({
        hostId: this.hostId,
        sessionId,
        terminalId,
        cols,
        rows,
        restart,
      });
    }
    throw new Error("Forge Anywhere only permits session and terminal streams");
  }
}

function inputUrl(input: RequestInfo | URL): URL {
  if (input instanceof URL) return input;
  if (typeof input === "string") return new URL(input);
  return new URL(input.url);
}

function assertHost(url: URL, hostId: string): void {
  if (!["fany:", "fany-ws:"].includes(url.protocol) || url.hostname !== hostId) {
    throw new Error("Forge Anywhere transport target does not match its enrolled host");
  }
}

function sessionPathParameter(encoded: string): string {
  let value: string;
  try {
    value = decodeURIComponent(encoded);
  } catch {
    throw new Error("invalid Forge Anywhere session path parameter");
  }
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(value)) {
    throw new Error("invalid Forge Anywhere session path parameter");
  }
  return value;
}

function routeFor(path: string, method: string): { route: BridgeRoute; parameters: string[] } {
  const exact: Record<string, Partial<Record<string, BridgeRoute>>> = {
    "/api/sessions": { GET: "list_sessions", POST: "create_session" },
    "/api/sessions/past": { GET: "past_sessions" },
    "/api/sessions/search": { GET: "search_sessions" },
    "/api/sessions/tree": { GET: "session_tree" },
    "/api/projects": { GET: "list_projects" },
    "/api/projects/browse": { GET: "browse_projects" },
    "/api/upload": { POST: "upload" },
    "/api/voice/transcribe": { POST: "voice_transcribe" },
    "/api/skills": { GET: "list_skills" },
    "/api/models": { GET: "list_models" },
    "/api/config": { GET: "read_config", PUT: "update_config" },
    "/api/hooks": { GET: "list_hooks" },
    "/api/plans": { GET: "list_plans" },
    "/api/mcp": { GET: "read_mcp", POST: "update_mcp" },
    "/api/usage": { GET: "usage" },
    "/api/diagnostics": { GET: "diagnostics" },
    "/api/history": { GET: "session_history" },
    "/api/answer": { POST: "answer" },
    "/api/push/key": { GET: "push_key" },
    "/api/push/subscribe": { POST: "push_subscribe" },
    "/api/push/unsubscribe": { POST: "push_unsubscribe" },
    "/api/terminals": { GET: "list_terminals" },
    "/api/git/status": { GET: "git_status" },
    "/api/git/branches": { GET: "git_branches" },
    "/api/git/diff": { GET: "git_diff" },
  };
  const route = exact[path]?.[method];
  if (route) return { route, parameters: [] };

  const session = path.match(/^\/api\/sessions\/([^/]+)\/(archive|fork|merge|discard)$/);
  if (session) {
    const operation: Record<string, BridgeRoute> = {
      archive: "archive_session",
      fork: "fork_session",
      merge: "merge_session",
      discard: "discard_session",
    };
    const expectedMethod = "POST";
    if (method === expectedMethod) {
      return { route: operation[session[2]], parameters: [sessionPathParameter(session[1])] };
    }
  }
  const sessionMetadata = path.match(/^\/api\/sessions\/([^/]+)$/);
  if (sessionMetadata) {
    const route =
      method === "PATCH"
        ? "rename_session"
        : method === "DELETE"
          ? "delete_session"
          : null;
    if (route) {
      return { route, parameters: [sessionPathParameter(sessionMetadata[1])] };
    }
  }
  throw new Error(`Forge Anywhere route is not allowlisted: ${method} ${path}`);
}
