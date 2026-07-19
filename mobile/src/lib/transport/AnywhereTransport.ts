import type { RemoteSocket, RemoteTransport } from "./RemoteTransport";

export type BridgeRoute =
  | "health"
  | "list_sessions"
  | "create_session"
  | "session_snapshot"
  | "session_history"
  | "session_input"
  | "archive_session"
  | "past_sessions"
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
  | "answer"
  | "push_key"
  | "push_subscribe"
  | "push_unsubscribe";

export interface AnywhereBridgeRequest {
  hostId: string;
  route: BridgeRoute;
  parameters: string[];
  method: string;
  headers: [string, string][];
  body: Uint8Array;
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
    const request = new Request("https://forge-anywhere.invalid/", init);
    const body = init?.body == null ? new Uint8Array() : new Uint8Array(await request.arrayBuffer());
    const response = await this.relay.request({
      hostId: this.hostId,
      route: mapping.route,
      parameters: [...mapping.parameters, url.search],
      method,
      headers: Array.from(request.headers.entries()),
      body,
    });
    return new Response(response.body as unknown as BodyInit, {
      status: response.status,
      headers: response.headers,
    });
  }

  openWebSocket(urlValue: string): RemoteSocket {
    const url = new URL(urlValue);
    assertHost(url, this.hostId);
    if (url.pathname !== "/ws") {
      throw new Error("Forge Anywhere only permits the /ws session stream");
    }
    const sessionId = url.searchParams.get("session");
    const revision = Number(url.searchParams.get("rev") ?? "0");
    if (!sessionId || !Number.isSafeInteger(revision) || revision < 0) {
      throw new Error("invalid Forge Anywhere session stream parameters");
    }
    return this.relay.openSessionSocket({
      hostId: this.hostId,
      sessionId,
      revision,
    });
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

function routeFor(path: string, method: string): { route: BridgeRoute; parameters: string[] } {
  const exact: Record<string, Partial<Record<string, BridgeRoute>>> = {
    "/api/sessions": { GET: "list_sessions", POST: "create_session" },
    "/api/sessions/past": { GET: "past_sessions" },
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
    "/api/history": { GET: "session_history" },
    "/api/answer": { POST: "answer" },
    "/api/push/key": { GET: "push_key" },
    "/api/push/subscribe": { POST: "push_subscribe" },
    "/api/push/unsubscribe": { POST: "push_unsubscribe" },
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
      return { route: operation[session[2]], parameters: [decodeURIComponent(session[1])] };
    }
  }
  throw new Error(`Forge Anywhere route is not allowlisted: ${method} ${path}`);
}
