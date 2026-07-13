// Typed HTTP client for the Forge daemon (`forge serve`). See BUILD_PLAN.md §1.2.
//
// Auth: the daemon token is a URL PATH SEGMENT, not a header — baseUrl already looks like
// `https://host:port/<token>`. Every function here takes `baseUrl` as its first argument;
// callers get it from `useAuth()` (src/lib/auth.tsx). No fetch happens outside this file
// (UI_RULES.md #3).
//
// Types mirror the serde struct field names VERBATIM (snake_case) — do not camelCase the
// wire (UI_RULES.md #4).

import { tFetch } from "./transport";

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

export interface ConfigResponse {
  fields: ConfigField[];
}

export interface ConfigField {
  key: string;
  group: string;
  field_type: "bool" | "int" | "float" | "enum" | "list" | "json" | "text";
  label: string;
  help: string | null;
  options: string[];
  value: string;
  default: string;
  modified: boolean;
  source: "project" | "user" | "default";
}

export interface UpdateConfigRequest {
  key: string;
  value?: string;
  scope: "user" | "project";
}

export interface SkillRow {
  name: string;
  description: string;
  scope: "builtin" | "user" | "project";
  tier: string | null;
  resources: number;
}

export interface UsageResponse {
  week: { sinceEpoch: number; combined: UsageTotals; providers: UsageProvider[] };
  session: { sessionId: string; combined: UsageTotals; providers: UsageProvider[] } | null;
  quota: UsageQuota[];
}
export interface UsageTotals { inputTokens: number; outputTokens: number; costUsd: number }
export interface UsageProvider extends UsageTotals { provider: string; kind: "bridge" | "oauth" | "api" }
export interface UsageQuota {
  provider: string;
  kind: "bridge" | "oauth" | "api";
  windowKind: string;
  status: string;
  resetsAt: number | null;
  fraction: number | null;
}
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
  temper?: "Read-only" | "Ask" | "Auto-edit" | "Full";
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

export interface TranscribeResponse {
  text: string;
}

export interface AnswerRequest {
  session: string;
  seq: number;
  allow: boolean;
}

export interface ErrorBody {
  error: string;
}

/** `GET /api/push/key` response — VAPID public key, base64url (§1.1). */
export interface PushKeyResponse {
  key: string;
}

/** Wire-verbatim mirror of the browser's `PushSubscription.toJSON()` shape. */
export interface PushSubscriptionKeys {
  p256dh: string;
  auth: string;
}

export interface WebPushSubscribeRequest {
  endpoint: string;
  keys: PushSubscriptionKeys;
}

/** A native device token (APNs) — the daemon's `SubscribeReq` discriminates by field
 * presence, not an explicit `kind` tag (see crates/forge-cli/src/serve.rs). */
export interface ApnsSubscribeRequest {
  device_token: string;
  environment: "sandbox" | "production";
}

/** A Live Activity's own push token — distinct from the general APNs device token above,
 * issued per-activity-instance (see crates/forge-cli/src/serve.rs's `SubscribeReq::LiveActivity`). */
export interface LiveActivitySubscribeRequest {
  session_id: string;
  push_token: string;
  environment: "sandbox" | "production";
}

export type PushSubscribeRequest =
  | WebPushSubscribeRequest
  | ApnsSubscribeRequest
  | LiveActivitySubscribeRequest;

export interface WebPushUnsubscribeRequest {
  endpoint: string;
}

export interface ApnsUnsubscribeRequest {
  device_token: string;
}

/** `push_token` is unused server-side for the delete (`session_id` is the key) but must still be
 * present — the untagged `SubscribeReq` discriminator matches on field presence. */
export interface LiveActivityUnsubscribeRequest {
  session_id: string;
  push_token: string;
}

export type PushUnsubscribeRequest =
  | WebPushUnsubscribeRequest
  | ApnsUnsubscribeRequest
  | LiveActivityUnsubscribeRequest;

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
    res = await tFetch(`${baseUrl}${path}`, {
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
    throw new ApiError(0, `server unreachable (${(err as Error)?.message ?? "network error"})`, err);
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

export function getConfig(baseUrl: string): Promise<ConfigResponse> {
  return request(baseUrl, "/api/config");
}

export function updateConfig(baseUrl: string, body: UpdateConfigRequest): Promise<ConfigResponse> {
  return request(baseUrl, "/api/config", {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

export function getSkills(baseUrl: string): Promise<SkillRow[]> {
  return request(baseUrl, "/api/skills");
}

export function getUsage(baseUrl: string, session?: string): Promise<UsageResponse> {
  return request(baseUrl, `/api/usage${qs({ session })}`);
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

export function transcribeAudio(
  baseUrl: string,
  form: FormData,
  language?: string,
): Promise<TranscribeResponse> {
  return request(baseUrl, `/api/voice/transcribe${qs({ language })}`, {
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

/** 503 (`ApiError.status === 503`) when the daemon has no VAPID key configured. */
export function getPushKey(baseUrl: string): Promise<PushKeyResponse> {
  return request(baseUrl, "/api/push/key");
}

export function subscribePush(
  baseUrl: string,
  body: PushSubscribeRequest,
): Promise<OkResponse> {
  return request(baseUrl, "/api/push/subscribe", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function unsubscribePush(
  baseUrl: string,
  body: PushUnsubscribeRequest,
): Promise<OkResponse> {
  return request(baseUrl, "/api/push/unsubscribe", {
    method: "POST",
    body: JSON.stringify(body),
  });
}
