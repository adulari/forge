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
