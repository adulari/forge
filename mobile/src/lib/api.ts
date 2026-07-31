// Typed HTTP client for the Forge daemon (`forge serve`). See BUILD_PLAN.md §1.2.
//
// Auth: the daemon token is a URL PATH SEGMENT, not a header — baseUrl already looks like
// `https://host:port/<token>`. Every function here takes `baseUrl` as its first argument;
// callers get it from `useAuth()` (src/lib/auth.tsx). No fetch happens outside this file
// (UI_RULES.md #3).
//
// Types mirror the serde struct field names VERBATIM (snake_case) — do not camelCase the
// wire (UI_RULES.md #4).

import { tFetch, TWebSocket } from "./transport";

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

export interface HostIdentity {
  hostname: string;
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

export interface SkillRow { name: string; description: string; scope: "builtin" | "user" | "project"; tier: string | null; resources: number; }

/** A declared workflow parameter (`meta.args`). `arg_type` is the author's free-form type word —
 * the wire field is not called `type` because that is a Rust keyword. */
export interface WorkflowArg { name: string; arg_type: string | null; required: boolean; description: string | null; default: string | null; }
/** One recorded run of a saved workflow, read from the `workflow_run` rows `run_saved_workflow`
 * writes — real runs only, never reconstructed.
 *
 * `status` is the honest verdict and the ONLY field that says whether a run is still going:
 * `finished_at` is null both while a run is live AND on a crash-interrupted one whose end moment
 * was never observed. `ok` is the coarse boolean (null for `running` and `interrupted`, which
 * reported no outcome at all). `phases`/`agents`/`cost_usd` are the run's own counts, closed out
 * when it finished — 0 on a run that is still going, and a real 0 on one that reported none. */
export interface WorkflowRun {
  started_at: number;
  finished_at: number | null;
  ok: boolean | null;
  summary: string | null;
  status: "running" | "ok" | "failed" | "interrupted";
  /** The session the run happened in, so its transcript can be opened. */
  session_id: string;
  phases: number;
  agents: number;
  cost_usd: number;
}
/** `runs` is newest-first and capped by the daemon (10 today). Empty for a workflow that has
 * never run on this machine, or that was only ever run inline via the `run_workflow` tool (which
 * authors an anonymous script belonging to no library entry). */
export interface WorkflowRow { name: string; description: string; when_to_use: string | null; phases: string[]; args: WorkflowArg[]; runs: WorkflowRun[]; }
export interface HookRow { event: string; matcher: string | null; command: string; timeout_secs: number; cc_compat: boolean; }
export interface ModelsResponse { catalog: "available" | "unavailable"; providers: ModelProvider[]; }
export interface ModelProvider { provider: string; models: ModelRow[]; }
export interface ModelRow { id: string; name: string; frontier: boolean; free: boolean; paid: boolean; subscription: boolean; estimated_cost_usd: number; health: { until_epoch: number; reason: string } | null; tier?: "trivial" | "standard" | "complex"; benchmark_intelligence?: number | null; benchmark_coding?: number | null; context_window?: number | null; }
export interface SessionTreeRow { id: string; title: string | null; forked_from: string | null; forked_at_seq: number | null; created_at: number; }
export interface PlanRow { session_id: string; session_title: string; title: string; steps: { title: string; detail: string }[]; notes: string | null; }

export interface CreateMcpServerRequest { name: string; transport: "stdio" | "http" | "sse"; command?: string; args?: string[]; url?: string; token_env?: string; }

export interface McpResponse { servers: McpServerRow[]; allowed_servers: string[]; allowed_tools: string[]; call_timeout_secs: number; connect_timeout_secs: number; }
// `editable` is false for a server the daemon resolves from somewhere other than the mcp.toml
// files it writes (an imported `.mcp.json`) — PATCH would 404, so the row stays read-only.
// Optional: a pre-v9 daemon omits it, and an absent flag must not disable a working toggle.
export interface McpServerRow { name: string; transport: "stdio" | "http" | "sse"; enabled: boolean; auth_configured: boolean; secret_env_count: number; editable?: boolean; }
/** `PATCH /api/mcp` — 404 when the server is configured somewhere the CLI does not write
 * (an imported `.mcp.json`); the daemon refuses to fork the source of truth. */
export interface UpdateMcpServerRequest { name: string; enabled: boolean; }

// --- Git review dock (crates/forge-cli/src/serve_git.rs) ---

/** `status` is git's porcelain letter for the bucket the row sits in: `M`/`A`/`D`/`R`/`C`/`T`/`U`
 * for tracked changes, `?` for untracked. A file edited after staging appears in BOTH buckets. */
export interface GitFileRow { path: string; status: string; orig_path: string | null; adds: number; dels: number; binary: boolean; }
export interface GitStatusResponse { root: string; branch: string; base_branch: string | null; staged: GitFileRow[]; unstaged: GitFileRow[]; untracked: GitFileRow[]; truncated: number; }
export interface GitBranchRow {
  name: string;
  oid: string;
  upstream: string | null;
  remote: boolean;
  current: boolean;
  default: boolean;
  worktree: string | null;
}
export interface GitBranchesResponse {
  root: string;
  current: string | null;
  default_branch: string | null;
  managed_worktree: boolean;
  actions_blocked_reason: string | null;
  branches: GitBranchRow[];
  truncated: number;
}
export interface GitSwitchRequest { session: string; branch: string; }
export interface GitCreateBranchRequest { session: string; name: string; }
export interface GitBranchActionResponse { ok: boolean; branch: string; }
/** Same shape as the WS snapshot's `DiffHunk` — the first character of each line is the gutter. */
export interface GitDiffHunk { header: string; lines: string[]; }
export interface GitDiffFile { path: string; kind: "created" | "modified" | "deleted" | "renamed"; orig_path: string | null; binary: boolean; adds: number; dels: number; hunks: GitDiffHunk[]; skipped_lines: number; }
export interface GitDiffResponse { root: string; staged: boolean; files: GitDiffFile[]; }
export interface GitPathsRequest { session: string; paths: string[]; }
export interface GitCommitRequest { session: string; message: string; }
export interface GitCommitResponse { ok: boolean; sha: string; summary: string; }
/** `GET /api/sessions/{id}/diff` — a fork's worktree against its `merge-base`, uncommitted edits
 * included. 400 for a session with no worktree. */
export interface SessionDiffResponse { base: string; branch: string; worktree: string; files: GitDiffFile[]; }

// --- Workspace inspector/editor (crates/forge-cli/src/serve_workspace.rs) ---

export type WorkspaceEntryKind = "directory" | "file" | "symlink";
export interface WorkspaceEntry { name: string; path: string; kind: WorkspaceEntryKind; size: number; modified_ms: number | null; }
export interface WorkspaceEntriesResponse { root: string; path: string; entries: WorkspaceEntry[]; truncated: number; }
export interface WorkspaceFileResponse { root: string; path: string; name: string; content: string; size: number; modified_ms: number | null; hash: string; extension: string | null; }
export type WorkspaceSearchMode = "files" | "content";
export interface WorkspaceSearchResult { path: string; kind: "file" | "match"; line: number | null; column: number | null; preview: string | null; }
export interface WorkspaceSearchResponse { query: string; mode: WorkspaceSearchMode; results: WorkspaceSearchResult[]; scanned_files: number; truncated: boolean; }
export interface WorkspaceWriteRequest { session: string; path: string; content: string; expected_hash: string; }

// --- Schedules (crates/forge-cli/src/serve_schedules.rs) ---

/** `cron` is the stored spec verbatim (`every:1800` / `daily:09:00` / `cron:<expr>`);
 * `spec_label` is the human rendering `forge schedule list` prints. */
export interface ScheduleRow { id: string; task: string; cwd: string; mode: string | null; model: string | null; cron: string; spec_label: string; enabled: boolean; created_at: number; last_run: number | null; }
/** Exactly one of `every` / `at` / `cron` must be set. */
export interface CreateScheduleRequest { task: string; cwd?: string; every?: string; at?: string; cron?: string; mode?: string; model?: string; }
export interface DeleteScheduleResponse { ok: true; deleted: true; }

// --- What's New (`GET /api/changelog`) ---

/** `section` is the `### Added` / `### Changed` / `### Fixed` heading the bullet sat under. */
export interface ChangelogEntry { section: string; text: string; }
/** `date` is null for the `Unreleased` section, which carries none. */
export interface ChangelogRelease { version: string; date: string | null; entries: ChangelogEntry[]; }

export interface UsageResponse {
  week: { sinceEpoch: number; combined: UsageTotals; providers: UsageProvider[] };
  session: { sessionId: string; combined: UsageTotals; providers: UsageProvider[] } | null;
  quota: UsageQuota[];
}
export interface UsageTotals { inputTokens: number; cachedInputTokens: number; outputTokens: number; costUsd: number }
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

export interface SessionSearchResult {
  id: string;
  title: string;
  cwd: string;
  archived: boolean;
  running: boolean;
  message_count: number;
  cost_usd: number;
  last_activity: number;
  match_source: "title" | "cwd" | "id" | "message";
  match_seq: number | null;
  match_role: "user" | "assistant" | null;
  match_excerpt: string | null;
}

/** Mirrors `TranscriptKind` in lib/ws.ts — same vocabulary on both wires, declared here so the
 * HTTP client keeps no dependency on the socket module. */
export type TranscriptKind = "user" | "assistant" | "tool" | "system";

export interface HistoryRow {
  seq: number;
  role: "user" | "assistant" | "system";
  content: string;
  model: string | null;
  created_at: number;
  visibility: "llm" | "ui";
  /** v9 additive: the row's provenance, so a paged-in replay row renders like a live transcript
   * row. Absent from a pre-v9 daemon — derive from `role` in that case. `"tool"` appears ONLY on
   * a page fetched with `include_tools`; without it the store selects just user/assistant turns
   * plus `ui` notes, leaving tool activity out of the stream. */
  kind?: TranscriptKind;
  /** The tool a `kind === "tool"` row belongs to — same field the live socket's transcript rows
   * use, so one renderer handles both. Null when the call carrier is no longer recoverable from
   * the store; absent on every page that asked for no tools. */
  tool?: string | null;
  /** Which half of the tool interaction the row is: `"call"` (the arguments the model sent, in
   * `content`) or `"result"` (what came back). The daemon omits the key entirely off an
   * `include_tools` page, and a pre-v10 daemon never sends it at all — absent means "render as
   * before", not "call". */
  tool_phase?: "call" | "result" | null;
  /** v9 additive: milliseconds from the session's FIRST visible row to this one — the zero point
   * a replay scrubber needs, which raw `created_at` epochs don't give. Second-resolution in fact
   * (`created_at` is stored in whole seconds). Absent from a pre-v9 daemon, null when the epoch
   * could not be read. */
  elapsed_ms?: number | null;
}

export interface CreateSessionRequest {
  cwd?: string;
  worktree?: boolean;
  title?: string;
  model?: string;
  resume?: string;
  temper?: "Read-only" | "Ask" | "Auto-edit" | "Full";
}

export interface ForkSessionRequest { at_seq: number; }

export interface CreateSessionResponse {
  id: string;
  title: string;
  cwd: string;
  worktree: string | null;
}

export interface ProjectRow {
  path: string;
  name: string;
  is_git_repo: boolean;
  last_activity: number | null;
}

export interface ProjectCatalog {
  default_cwd: string;
  recent: ProjectRow[];
  roots: ProjectRow[];
}

export interface BrowseProjectsResponse {
  path: string;
  parent: string | null;
  entries: ProjectRow[];
  roots: ProjectRow[];
  truncated: boolean;
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

const DEFAULT_REQUEST_TIMEOUT_MS = 15_000;
const LARGE_UPLOAD_TIMEOUT_MS = 120_000;

async function request<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
  timeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
): Promise<T> {
  const controller = new AbortController();
  const callerSignal = init?.signal;
  let timedOut = false;

  const abortFromCaller = () => controller.abort(callerSignal?.reason);
  if (callerSignal?.aborted) abortFromCaller();
  else callerSignal?.addEventListener("abort", abortFromCaller, { once: true });

  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort(new Error(`request timed out after ${timeoutMs}ms`));
  }, timeoutMs);

  let res: Response;
  try {
    res = await tFetch(`${baseUrl}${path}`, {
      ...init,
      signal: controller.signal,
      headers: {
        Accept: "application/json",
        ...(init?.body && !(init.body instanceof FormData)
          ? { "Content-Type": "application/json" }
          : {}),
        ...init?.headers,
      },
    });
  } catch (err) {
    if (timedOut) {
      clearTimeout(timeout);
      callerSignal?.removeEventListener("abort", abortFromCaller);
      throw new ApiError(0, `request timed out after ${Math.round(timeoutMs / 1000)}s`, err);
    }
    clearTimeout(timeout);
    callerSignal?.removeEventListener("abort", abortFromCaller);
    throw new ApiError(0, `server unreachable (${(err as Error)?.message ?? "network error"})`, err);
  }

  try {
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
  } catch (err) {
    if (timedOut) {
      throw new ApiError(0, `request timed out after ${Math.round(timeoutMs / 1000)}s`, err);
    }
    throw err;
  } finally {
    clearTimeout(timeout);
    callerSignal?.removeEventListener("abort", abortFromCaller);
  }
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

export function forkSession(baseUrl: string, id: string, body: ForkSessionRequest): Promise<CreateSessionResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}/fork`, { method: "POST", body: JSON.stringify(body) });
}

export function getSessions(baseUrl: string): Promise<SessionRow[]> {
  return request(baseUrl, "/api/sessions");
}

export function getProjects(baseUrl: string): Promise<ProjectCatalog> {
  return request(baseUrl, "/api/projects");
}

export function browseProjects(baseUrl: string, path?: string): Promise<BrowseProjectsResponse> {
  return request(baseUrl, `/api/projects/browse${qs({ path })}`);
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

export function getSessionTree(baseUrl: string): Promise<SessionTreeRow[]> {
  return request(baseUrl, "/api/sessions/tree");
}

export function getPastSessions(
  baseUrl: string,
  params: { limit?: number; before?: number } = {},
): Promise<PastSessionRow[]> {
  return request(baseUrl, `/api/sessions/past${qs(params)}`);
}

export function searchSessions(
  baseUrl: string,
  query: string,
  limit = 30,
): Promise<SessionSearchResult[]> {
  return request(baseUrl, `/api/sessions/search${qs({ q: query, limit })}`);
}

export function renameSession(
  baseUrl: string,
  id: string,
  title: string,
): Promise<OkResponse & { title: string }> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify({ title }),
  });
}

export function deleteSession(baseUrl: string, id: string): Promise<OkResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
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

export function getSkills(baseUrl: string): Promise<SkillRow[]> {
  return request(baseUrl, "/api/skills");
}

export function getWorkflows(baseUrl: string, session?: string): Promise<WorkflowRow[]> {
  return request(baseUrl, `/api/workflows${qs({ session })}`);
}

export function getModels(baseUrl: string): Promise<ModelsResponse> {
  return request(baseUrl, "/api/models");
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

export function getHooks(baseUrl: string): Promise<HookRow[]> { return request(baseUrl, "/api/hooks"); }
export function getPlans(baseUrl: string): Promise<PlanRow[]> { return request(baseUrl, "/api/plans"); }

export function createMcpServer(baseUrl: string, body: CreateMcpServerRequest): Promise<McpResponse> { return request(baseUrl, "/api/mcp", { method: "POST", body: JSON.stringify(body) }); }

export function getMcp(baseUrl: string): Promise<McpResponse> { return request(baseUrl, "/api/mcp"); }

export function updateMcpServer(baseUrl: string, body: UpdateMcpServerRequest): Promise<McpResponse> {
  return request(baseUrl, "/api/mcp", { method: "PATCH", body: JSON.stringify(body) });
}

// ---------------------------------------------------------------------------
// Git review dock
// ---------------------------------------------------------------------------

export function getGitBranches(
  baseUrl: string,
  session: string,
  query = "",
  limit = 500,
): Promise<GitBranchesResponse> {
  return request(baseUrl, `/api/git/branches${qs({ session, q: query, limit })}`);
}

export function switchGitBranch(
  baseUrl: string,
  body: GitSwitchRequest,
): Promise<GitBranchActionResponse> {
  return request(baseUrl, "/api/git/switch", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function createGitBranch(
  baseUrl: string,
  body: GitCreateBranchRequest,
): Promise<GitBranchActionResponse> {
  return request(baseUrl, "/api/git/branches", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export function getGitStatus(baseUrl: string, session: string): Promise<GitStatusResponse> {
  return request(baseUrl, `/api/git/status${qs({ session })}`);
}

/** `staged` picks which side of the index is diffed: `true` = index vs HEAD, `false` = working
 * tree vs index (with an all-additions render for untracked files). */
export function getGitDiff(
  baseUrl: string,
  params: { session: string; path: string; staged?: boolean },
): Promise<GitDiffResponse> {
  return request(
    baseUrl,
    `/api/git/diff${qs({ session: params.session, path: params.path, staged: params.staged ? "true" : "false" })}`,
  );
}

export function stagePaths(baseUrl: string, body: GitPathsRequest): Promise<OkResponse> {
  return request(baseUrl, "/api/git/stage", { method: "POST", body: JSON.stringify(body) });
}

export function unstagePaths(baseUrl: string, body: GitPathsRequest): Promise<OkResponse> {
  return request(baseUrl, "/api/git/unstage", { method: "POST", body: JSON.stringify(body) });
}

/** Commits ONLY what is already staged — the dock's staging decisions are the whole contract. */
export function commitStaged(baseUrl: string, body: GitCommitRequest): Promise<GitCommitResponse> {
  return request(baseUrl, "/api/git/commit", { method: "POST", body: JSON.stringify(body) });
}

export function getSessionDiff(baseUrl: string, id: string): Promise<SessionDiffResponse> {
  return request(baseUrl, `/api/sessions/${encodeURIComponent(id)}/diff`);
}

// ---------------------------------------------------------------------------
// Workspace inspector/editor
// ---------------------------------------------------------------------------

export function getWorkspaceEntries(
  baseUrl: string,
  session: string,
  path = "",
): Promise<WorkspaceEntriesResponse> {
  return request(baseUrl, `/api/workspace/entries${qs({ session, path })}`);
}

export function getWorkspaceFile(
  baseUrl: string,
  session: string,
  path: string,
): Promise<WorkspaceFileResponse> {
  return request(baseUrl, `/api/workspace/file${qs({ session, path })}`);
}

export function searchWorkspace(
  baseUrl: string,
  params: { session: string; q: string; mode?: WorkspaceSearchMode; limit?: number },
): Promise<WorkspaceSearchResponse> {
  return request(
    baseUrl,
    `/api/workspace/search${qs({
      session: params.session,
      q: params.q,
      mode: params.mode ?? "files",
      limit: params.limit ?? 50,
    })}`,
  );
}

export function writeWorkspaceFile(
  baseUrl: string,
  body: WorkspaceWriteRequest,
): Promise<WorkspaceFileResponse> {
  return request(baseUrl, "/api/workspace/file", {
    method: "PUT",
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Schedules
// ---------------------------------------------------------------------------

export function getSchedules(baseUrl: string): Promise<ScheduleRow[]> {
  return request(baseUrl, "/api/schedules");
}

export function createSchedule(baseUrl: string, body: CreateScheduleRequest): Promise<ScheduleRow> {
  return request(baseUrl, "/api/schedules", { method: "POST", body: JSON.stringify(body) });
}

export function pauseSchedule(baseUrl: string, id: string): Promise<ScheduleRow> {
  return request(baseUrl, `/api/schedules/${encodeURIComponent(id)}/pause`, { method: "POST" });
}

export function resumeSchedule(baseUrl: string, id: string): Promise<ScheduleRow> {
  return request(baseUrl, `/api/schedules/${encodeURIComponent(id)}/resume`, { method: "POST" });
}

export function deleteSchedule(baseUrl: string, id: string): Promise<DeleteScheduleResponse> {
  return request(baseUrl, `/api/schedules/${encodeURIComponent(id)}/delete`, { method: "POST" });
}

// ---------------------------------------------------------------------------
// What's New
// ---------------------------------------------------------------------------

export function getChangelog(baseUrl: string, limit?: number): Promise<ChangelogRelease[]> {
  return request(baseUrl, `/api/changelog${qs({ limit })}`);
}

export function getUsage(baseUrl: string, session?: string): Promise<UsageResponse> {
  return request(baseUrl, `/api/usage${qs({ session })}`);
}

/** `include_tools` widens the page to tool CALL and RESULT rows (`tool_phase` says which).
 * Opt-in: the parameter is left off the URL entirely unless asked for, so every existing caller
 * sends the same request and gets the same payload it always did. */
export function getHistory(
  baseUrl: string,
  params: { session: string; before?: number; limit?: number; include_tools?: boolean },
): Promise<HistoryRow[]> {
  const { include_tools, ...page } = params;
  return request(
    baseUrl,
    `/api/history${qs({ ...page, include_tools: include_tools ? 1 : undefined })}`,
  );
}

export function uploadFile(
  baseUrl: string,
  sessionId: string,
  form: FormData,
): Promise<UploadResponse> {
  return request(
    baseUrl,
    `/api/upload${qs({ session: sessionId })}`,
    { method: "POST", body: form },
    LARGE_UPLOAD_TIMEOUT_MS,
  );
}

export function transcribeAudio(
  baseUrl: string,
  form: FormData,
  language?: string,
): Promise<TranscribeResponse> {
  return request(
    baseUrl,
    `/api/voice/transcribe${qs({ language })}`,
    { method: "POST", body: form },
    LARGE_UPLOAD_TIMEOUT_MS,
  );
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

export function getIdentity(baseUrl: string): Promise<HostIdentity> {
  return request(baseUrl, "/api/identity");
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

// ---------------------------------------------------------------------------
// Terminal dock (WS /ws/terminal)
// ---------------------------------------------------------------------------

/** Client → server frames on `/ws/terminal`. */
export type TerminalClientFrame =
  | { kind: "input"; data: string }
  | { kind: "resize"; cols: number; rows: number }
  | { kind: "clear" }
  | { kind: "close" };

export type TerminalStatus = "running" | "exited";

export interface TerminalSessionSummary {
  terminal_id: string;
  status: TerminalStatus;
  clients: number;
  updated_at_ms: number;
}

type TerminalServerFrame =
  | { kind: "status"; status: TerminalStatus }
  | { kind: "cleared" };

export interface TerminalSocketHandlers {
  /** Decoded pty output. UTF-8 is decoded in STREAMING mode, so a multi-byte glyph split across
   * two frames is reassembled instead of turning into replacement characters. */
  onOutput: (chunk: string) => void;
  onStatus?: (status: TerminalStatus) => void;
  onClear?: () => void;
  onOpen?: () => void;
  onClose?: (event: { code: number; reason: string }) => void;
  onError?: (error: unknown) => void;
}

export interface TerminalSocketOptions {
  terminalId?: string;
  size?: { cols: number; rows: number };
  restart?: boolean;
}

export interface TerminalSocket {
  send: (data: string) => boolean;
  resize: (cols: number, rows: number) => boolean;
  clear: () => boolean;
  kill: () => boolean;
  /** Detach this client without stopping the daemon-owned terminal process. */
  close: () => void;
}

export function listTerminalSessions(
  baseUrl: string,
  sessionId: string,
): Promise<TerminalSessionSummary[]> {
  return request(baseUrl, `/api/terminals${qs({ session: sessionId })}`);
}

/** Attach to a daemon-owned PTY in a session's directory. The process survives socket detach. */
export function openTerminalSocket(
  baseUrl: string,
  sessionId: string,
  handlers: TerminalSocketHandlers,
  options: TerminalSocketOptions = {},
): TerminalSocket {
  const url = new URL(`${baseUrl}/ws/terminal`);
  url.protocol =
    url.protocol === "fany:" ? "fany-ws:" : url.protocol === "https:" ? "wss:" : "ws:";
  url.searchParams.set("session", sessionId);
  url.searchParams.set("terminal", options.terminalId ?? "term-1");
  if (options.size) {
    url.searchParams.set("cols", String(options.size.cols));
    url.searchParams.set("rows", String(options.size.rows));
  }
  if (options.restart) url.searchParams.set("restart", "true");

  const socket = new TWebSocket(url.toString());
  socket.binaryType = "arraybuffer";
  const decoder = new TextDecoder();

  socket.onopen = () => handlers.onOpen?.();
  socket.onerror = (error: unknown) => handlers.onError?.(error);
  socket.onclose = (event: { code?: number; reason?: string }) =>
    handlers.onClose?.({ code: event.code ?? 0, reason: event.reason ?? "" });
  socket.onmessage = (event: MessageEvent) => {
    const { data } = event;
    if (typeof data === "string") {
      const control = parseTerminalServerFrame(data);
      if (control?.kind === "status") {
        handlers.onStatus?.(control.status);
        return;
      }
      if (control?.kind === "cleared") {
        handlers.onClear?.();
        return;
      }
      handlers.onOutput(data);
      return;
    }
    handlers.onOutput(decoder.decode(new Uint8Array(data as ArrayBuffer), { stream: true }));
  };

  const post = (frame: TerminalClientFrame): boolean => {
    if (socket.readyState !== TWebSocket.OPEN) return false;
    socket.send(JSON.stringify(frame));
    return true;
  };

  return {
    send: (data: string) => post({ kind: "input", data }),
    resize: (cols: number, rows: number) => post({ kind: "resize", cols, rows }),
    clear: () => post({ kind: "clear" }),
    kill: () => post({ kind: "close" }),
    close: () => socket.close(),
  };
}

function parseTerminalServerFrame(value: string): TerminalServerFrame | null {
  try {
    const frame = JSON.parse(value) as Partial<TerminalServerFrame>;
    if (frame.kind === "cleared") return { kind: "cleared" };
    if (
      frame.kind === "status"
      && (frame.status === "running" || frame.status === "exited")
    ) {
      return { kind: "status", status: frame.status };
    }
  } catch {
    // A non-control text frame is a human-readable daemon error.
  }
  return null;
}
