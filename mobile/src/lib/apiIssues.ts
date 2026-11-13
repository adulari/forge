// GitHub issues over `/api/issues*` (serve_issues.rs), a thin layer over the host's `gh`. Unlike
// the PR routes these are bridged, so they also work through Forge Anywhere. A missing or
// signed-out `gh` arrives as `available: false` on the list rather than as an error.
import { request } from "./api";
import type { PrWatch, PrWatchMethod, PrWatchMode } from "./apiPr";

export interface IssueLabel {
  name: string;
  color: string;
}

export interface IssueRow {
  number: number;
  title: string;
  url: string;
  state: "open" | "closed";
  labels: IssueLabel[];
  author: string;
  created_at: string;
  updated_at: string;
  comment_count: number;
  repo: string;
}

export interface IssueListResponse {
  available: boolean;
  error: string | null;
  issues: IssueRow[];
}

export interface IssueDetailResponse {
  number: number;
  title: string;
  body: string;
  url: string;
  author: string;
  labels: IssueLabel[];
  comments: { author: string; body: string; created_at: string }[];
}

export type IssueState = "open" | "closed" | "all";

export interface IssueStartRequest {
  session: string;
  number: number;
  worktree: boolean;
  mode: PrWatchMode;
  method?: PrWatchMethod;
}

export interface IssueStartResponse {
  id: string;
  title: string;
  cwd: string;
  worktree: string | null;
  issue: number;
  watch: PrWatch | null;
}

const enc = encodeURIComponent;

export const getIssueList = (baseUrl: string, session: string, state: IssueState) =>
  request<IssueListResponse>(baseUrl, `/api/issues?session=${enc(session)}&state=${state}`);

export const getIssueDetail = (baseUrl: string, session: string, number: number) =>
  request<IssueDetailResponse>(baseUrl, `/api/issues/detail?session=${enc(session)}&number=${number}`);

// Creating the worktree and the session can take a while on a big repository.
export const startIssue = (baseUrl: string, body: IssueStartRequest) =>
  request<IssueStartResponse>(baseUrl, "/api/issues/start", { method: "POST", body: JSON.stringify(body) }, 90_000);

export type DrainItemStatus =
  | "queued"
  | "running"
  | "waiting"
  | "pr_open"
  | "merged"
  | "done"
  | "failed"
  | "blocked"
  | "cancelled";

export interface DrainItem {
  number: number;
  status: DrainItemStatus;
  session_id: string | null;
  title: string | null;
  pr_url: string | null;
  outcome: string | null;
}

export interface DrainEnvelope {
  meter: string;
  label: string;
  fill: number;
  spent: boolean;
  unverified: boolean;
  resume_label: string | null;
}

export interface IssueDrain {
  id: string;
  cwd: string;
  status: "active" | "paused" | "finished" | "cancelled";
  parallel: number;
  budget: string | null;
  paused_reason: string | null;
  created_at: number;
  updated_at: number;
  /** `Drain: 7 started, 4 merged, 2 waiting on you, 1 blocked`. */
  line: string;
  envelope: DrainEnvelope | null;
  items: DrainItem[];
}

export interface IssueDrainRequest {
  session: string;
  numbers: number[];
  parallel: number;
  /** `claude:30%w`, `codex:15%w` or `$5`; omitted for no envelope. */
  budget?: string;
  worktree: boolean;
  mode: PrWatchMode;
  method?: PrWatchMethod;
}

export const getIssueDrains = (baseUrl: string) =>
  request<{ drains: IssueDrain[] }>(baseUrl, "/api/issues/drains");

// The daemon reads the quota window before it answers, which can take a few seconds.
export const startIssueDrain = (baseUrl: string, body: IssueDrainRequest) =>
  request<IssueDrain>(baseUrl, "/api/issues/drain", { method: "POST", body: JSON.stringify(body) }, 45_000);

export const cancelIssueDrain = (baseUrl: string, id: string) =>
  request<IssueDrain>(baseUrl, "/api/issues/drain/cancel", { method: "POST", body: JSON.stringify({ id }) });
