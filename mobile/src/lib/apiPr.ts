// Pull requests and GitHub Actions over `/api/pr/*` and `/api/actions/*` (serve_pr.rs,
// serve_actions.rs). Both are thin layers over the host's `gh`; a missing or signed-out `gh`
// arrives as `available: false` on the read routes rather than as an error.
import { request, type GitDiffFile } from "./api";

export type CheckBucket = "pending" | "passing" | "failing";

export interface PrCheckItem {
  name: string;
  bucket: CheckBucket;
  detail: string;
  url: string | null;
  workflow: string | null;
}

export interface PrChecks {
  total: number;
  passing: number;
  failing: number;
  pending: number;
  items: PrCheckItem[];
}

export interface PrSummary {
  number: number;
  url: string;
  title: string;
  state: "open" | "closed" | "merged";
  draft: boolean;
  head_ref: string;
  base_ref: string;
  head_sha: string;
  mergeable: string;
  merge_state: string;
  review_decision: string;
  additions: number;
  deletions: number;
  changed_files: number;
  checks: PrChecks;
}

export interface PrStatusResponse {
  available: boolean;
  error: string | null;
  branch: string | null;
  pr: PrSummary | null;
}

export interface PrListRow extends PrSummary {
  author: string;
  updated_at: string;
  repo: string;
}

export interface PrListResponse {
  available: boolean;
  error: string | null;
  prs: PrListRow[];
}

export interface PrDetailResponse {
  pr: PrSummary;
  body: string;
  author: string;
  comments: { author: string; body: string; created_at: string }[];
  commits: { sha: string; headline: string }[];
  files: { path: string; additions: number; deletions: number }[];
}

export interface PrDiffResponse {
  files: GitDiffFile[];
  whitespace_hidden: boolean;
  commit: string | null;
}

export type PrState = "open" | "closed" | "merged" | "all";
export type MergeMethod = "squash" | "merge" | "rebase";

export interface PrCreateRequest {
  session: string;
  title: string;
  body?: string;
  base?: string;
  draft?: boolean;
}

export interface PrMergeRequest {
  session: string;
  number: number;
  head_sha: string;
  method?: MergeMethod;
  delete_branch?: boolean;
}

export interface PrMergeResponse {
  ok: boolean;
  sync: "pulled" | "fetched" | "skipped";
  base: string;
}

function query(params: Record<string, string | number | boolean | undefined>): string {
  const parts = Object.entries(params)
    .filter(([, value]) => value !== undefined && value !== "")
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(String(value))}`);
  return parts.length > 0 ? `?${parts.join("&")}` : "";
}

function post<T>(baseUrl: string, path: string, body: unknown, timeoutMs?: number): Promise<T> {
  return request<T>(baseUrl, path, { method: "POST", body: JSON.stringify(body) }, timeoutMs);
}

export const getPrStatus = (baseUrl: string, session: string) =>
  request<PrStatusResponse>(baseUrl, `/api/pr/status${query({ session })}`);

export const getPrList = (baseUrl: string, session: string, state: PrState) =>
  request<PrListResponse>(baseUrl, `/api/pr/list${query({ session, state })}`);

export const getPrDetail = (baseUrl: string, session: string, number: number) =>
  request<PrDetailResponse>(baseUrl, `/api/pr/detail${query({ session, number })}`);

export const getPrDiff = (
  baseUrl: string,
  params: { session: string; number: number; commit?: string | null; ignoreWhitespace?: boolean },
) =>
  request<PrDiffResponse>(
    baseUrl,
    `/api/pr/diff${query({
      session: params.session,
      number: params.number,
      commit: params.commit ?? undefined,
      ignore_whitespace: params.ignoreWhitespace ? true : undefined,
    })}`,
    undefined,
    45_000,
  );

export const createPr = (baseUrl: string, body: PrCreateRequest) =>
  post<{ ok: boolean; url: string | null; output: string }>(baseUrl, "/api/pr/create", body, 180_000);

export const commentOnPr = (baseUrl: string, body: { session: string; number: number; body: string }) =>
  post<{ ok: boolean }>(baseUrl, "/api/pr/comment", body);

export const mergePr = (baseUrl: string, body: PrMergeRequest) =>
  post<PrMergeResponse>(baseUrl, "/api/pr/merge", body, 90_000);

export type RunStatus = "queued" | "in_progress" | "completed" | "waiting" | "requested" | "pending";

export interface WorkflowRow {
  id: number;
  name: string;
  path: string;
  state: string;
}

export interface WorkflowRunRow {
  databaseId: number;
  displayTitle: string;
  workflowName: string;
  status: RunStatus;
  conclusion: string;
  headBranch: string;
  headSha: string;
  event: string;
  createdAt: string;
  updatedAt: string;
  url: string;
  number: number;
}

export interface RunStep {
  name: string;
  status: string;
  conclusion: string;
  number: number;
}

export interface RunJob {
  name: string;
  status: string;
  conclusion: string;
  url: string;
  steps: RunStep[];
}

export interface RunDetail extends Omit<WorkflowRunRow, "headSha" | "number"> {
  jobs: RunJob[];
}

export interface DispatchInput {
  name: string;
  description: string;
  required: boolean;
  default: string | null;
  kind: string;
  options: string[];
}

export interface ActionsList<T> {
  available: boolean;
  error: string | null;
  items: T[];
}

export const getWorkflows = (baseUrl: string, session: string) =>
  request<ActionsList<WorkflowRow>>(baseUrl, `/api/actions/workflows${query({ session })}`);

export const getWorkflowRuns = (baseUrl: string, session: string, workflow?: string) =>
  request<ActionsList<WorkflowRunRow>>(baseUrl, `/api/actions/runs${query({ session, workflow, limit: 30 })}`);

export const getRunDetail = (baseUrl: string, session: string, id: number) =>
  request<RunDetail>(baseUrl, `/api/actions/run${query({ session, id })}`);

export const getDispatchInputs = (baseUrl: string, session: string, workflow: string) =>
  request<{ inputs: DispatchInput[] }>(baseUrl, `/api/actions/inputs${query({ session, workflow })}`);

export const dispatchWorkflow = (
  baseUrl: string,
  body: { session: string; workflow: string; ref?: string; inputs?: Record<string, string | boolean> },
) => post<{ ok: boolean }>(baseUrl, "/api/actions/dispatch", body);

export const runAction = (
  baseUrl: string,
  verb: "rerun" | "cancel" | "delete",
  body: { session: string; id: number; failed_only?: boolean },
) => post<{ ok: boolean }>(baseUrl, `/api/actions/${verb}`, body);

export type PrWatchMode = "off" | "fix" | "merge";
export type PrWatchMethod = MergeMethod | "auto";

export interface PrWatch {
  session_id: string;
  mode: PrWatchMode;
  method: PrWatchMethod;
  state: "watching" | "merged" | "blocked";
  blocked_reason: string | null;
  pr_number: number | null;
  pr_url: string | null;
  head_sha: string | null;
  fix_turns: number;
  last_checked: number | null;
  last_action: string | null;
  created_at: number;
}

export const getPrWatch = (baseUrl: string, session: string) =>
  request<{ watch: PrWatch | null }>(baseUrl, `/api/pr/watch${query({ session })}`);

export const getPrWatches = (baseUrl: string) => request<{ watches: PrWatch[] }>(baseUrl, "/api/pr/watches");

export const setPrWatch = (baseUrl: string, body: { session: string; mode: PrWatchMode; method?: PrWatchMethod }) =>
  post<{ watch: PrWatch | null }>(baseUrl, "/api/pr/watch", body);
