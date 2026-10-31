import { request } from "./api";
import type { AddHostJob, AddedHost } from "./hostAddCore";

export * from "./hostAddCore";

export function listAddedHosts(baseUrl: string): Promise<{ hosts: AddedHost[] }> {
  return request(baseUrl, "/api/hosts");
}

export function startAddHost(
  baseUrl: string,
  body: { target: string; name?: string; lan?: boolean; pair?: boolean },
): Promise<{ job_id: string }> {
  return request(baseUrl, "/api/hosts/add", { method: "POST", body: JSON.stringify(body) });
}

export function getAddHostJob(baseUrl: string, id: string): Promise<AddHostJob> {
  return request(baseUrl, `/api/hosts/jobs/${encodeURIComponent(id)}`);
}

export function answerAddHostApproval(baseUrl: string, id: string, approve: boolean): Promise<{ ok: true }> {
  return request(baseUrl, `/api/hosts/jobs/${encodeURIComponent(id)}/approve`, {
    method: "POST",
    body: JSON.stringify({ approve }),
  });
}
