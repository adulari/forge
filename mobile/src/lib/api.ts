// Typed HTTP client for the Forge daemon (`forge serve`). See BUILD_PLAN.md §1.2.
//
// Auth: the daemon token is a URL PATH SEGMENT, not a header — baseUrl already looks like
// `https://host:port/<token>`. Every function here takes `baseUrl` as its first argument;
// callers get it from `useAuth()` (src/lib/auth.tsx). No fetch happens outside this file
// (UI_RULES.md #3).
//
// Types mirror the serde struct field names VERBATIM (snake_case) — do not camelCase the
// wire (UI_RULES.md #4).

export class ApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, message: string, body?: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

// ---------------------------------------------------------------------------
// Wire types (verbatim field names)
// ---------------------------------------------------------------------------

export interface SessionRow {
  id: string;
  title: string;
  cwd: string;
  worktree: string | null;
  busy: boolean;
  waiting: boolean;
  cost_usd: number;
  context_tokens: number;
  context_limit: number | null;
  model: string;
  created_at: number;
  last_activity: number;
}

export interface PastSessionRow {
  id: string;
  title: string;
  cwd: string;
  worktree: string | null;
  archived: boolean;
  message_count: number;
  cost_usd: number;
  last_activity: number;
  created_at: number;
  preview: string | null;
}

export interface HistoryRow {
  seq: number;
  role: "user" | "assistant" | "system";
  content: string;
  model: string | null;
  created_at: number;
  visibility: "llm" | "ui";
}

export interface CreateSessionRequest {
  cwd?: string;
  worktree?: boolean;
  title?: string;
  model?: string;
  resume?: string;
}

export interface CreateSessionResponse {
  id: string;
  title: string;
  cwd: string;
  worktree: string | null;
}

export interface OkResponse {
  ok: true;
}

export interface MergeSuccessResponse {
  ok: true;
  merged: true;
  branch: string;
}

export interface MergeDirtyConflictResponse {
  error: string;
  dirty_files?: string[];
  conflicts?: string[];
  branch?: string;
  worktree?: string;
}

export interface DiscardResponse {
  ok: true;
  discarded: true;
  branch: string;
  warnings: string[];
}

export interface UploadedFile {
  name: string;
  path: string;
  image: boolean;
}

export interface UploadResponse {
  files: UploadedFile[];
}

export interface AnswerRequest {
  session: string;
  seq: number;
  allow: boolean;
}

export interface ErrorBody {
  error: string;
}

// ---------------------------------------------------------------------------
// Fetch wrapper
// ---------------------------------------------------------------------------

async function request<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${baseUrl}${path}`, {
      ...init,
      headers: {
        Accept: "application/json",
        ...(init?.body && !(init.body instanceof FormData)
          ? { "Content-Type": "application/json" }
          : {}),
        ...init?.headers,
      },
    });
  } catch (err) {
    throw new ApiError(0, "server unreachable", err);
  }

  if (res.status === 404) {
    // Deliberately unrevealing per §1.1 — could be wrong token OR a real 404 route.
    let body: unknown;
    try {
      body = await res.json();
    } catch {
      // no body
    }
    throw new ApiError(404, "pairing invalid, re-scan", body);
  }

  if (!res.ok) {
    let body: ErrorBody | undefined;
    try {
      body = (await res.json()) as ErrorBody;
    } catch {
      // no body
    }
    throw new ApiError(res.status, body?.error ?? `request failed (${res.status})`, body);
  }

  if (res.status === 204) {
    return undefined as T;
  }

  return (await res.json()) as T;
}

function qs(params: Record<string, string | number | undefined>): string {
  const entries = Object.entries(params).filter(([, v]) => v !== undefined);
  if (entries.length === 0) return "";
  const search = new URLSearchParams();
  for (const [k, v] of entries) search.set(k, String(v));
  return `?${search.toString()}`;
}

// ---------------------------------------------------------------------------
// Endpoints (§1.2)
// ---------------------------------------------------------------------------

export function getSessions(baseUrl: string): Promise<SessionRow[]> {
  return request(baseUrl, "/api/sessions");
}

export function createSession(
  baseUrl: string,
  body: CreateSessionRequest,
): Promise<CreateSessionResponse> {
  return request(baseUrl, "/api/sessions", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function getPastSessions(
  baseUrl: string,
  params: { limit?: number; before?: number } = {},
): Promise<PastSessionRow[]> {
  return request(baseUrl, `/api/sessions/past${qs(params)}`);
}

export function archiveSession(baseUrl: string, id: string): Promise<OkResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}/archive`, {
    method: "POST",
  });
}

export function mergeSession(
  baseUrl: string,
  id: string,
): Promise<MergeSuccessResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}/merge`, {
    method: "POST",
  });
}

export function discardSession(
  baseUrl: string,
  id: string,
): Promise<DiscardResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}/discard`, {
    method: "POST",
  });
}

export function getHistory(
  baseUrl: string,
  params: { session: string; before?: number; limit?: number },
): Promise<HistoryRow[]> {
  return request(baseUrl, `/api/history${qs(params)}`);
}

export function uploadFile(
  baseUrl: string,
  sessionId: string,
  form: FormData,
): Promise<UploadResponse> {
  return request(baseUrl, `/api/upload${qs({ session: sessionId })}`, {
    method: "POST",
    body: form,
  });
}

export function answer(baseUrl: string, body: AnswerRequest): Promise<OkResponse> {
  return request(baseUrl, "/api/answer", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

/** Connectivity/auth probe — 200 array = good token, 404 = wrong token. */
export function probeConnection(baseUrl: string): Promise<SessionRow[]> {
  return getSessions(baseUrl);
}
