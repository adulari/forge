// Per-client tokens and Remote Access over `/api/remote-access`, `/api/clients/*`,
// `/api/whoami` and `/api/ws-ticket` (serve_access.rs). Direct daemon connections only: the
// Anywhere bridge has its own device list.
import { request } from "./api";

export type AccessScope = "view" | "prompt" | "approve" | "terminal" | "workspace-write" | "manage-providers";
export type AccessPreset = "full" | "viewer";
export type AddressKind = "local" | "lan" | "tailscale" | "public";

export interface WhoAmI {
  id: string | null;
  label: string;
  owner: boolean;
  scopes: AccessScope[];
}

export interface PairedClient {
  id: string;
  label: string;
  scopes: AccessScope[];
  created_at: number;
  last_seen: number | null;
  pending: boolean;
  preset: AccessPreset;
}

export interface AccessAddress {
  kind: AddressKind;
  base: string | null;
  note: string;
}

export interface RemoteAccessInfo {
  online: boolean;
  mode: string;
  addresses: AccessAddress[];
  clients: PairedClient[];
}

export interface PairResult {
  client: PairedClient;
  token: string;
  expires_in: number;
}

export interface PairRequest {
  label?: string;
  preset?: AccessPreset;
  ttl_secs?: number;
}

export function getWhoAmI(baseUrl: string): Promise<WhoAmI> {
  return request<WhoAmI>(baseUrl, "/api/whoami");
}

export function getRemoteAccess(baseUrl: string): Promise<RemoteAccessInfo> {
  return request<RemoteAccessInfo>(baseUrl, "/api/remote-access");
}

export function pairDevice(baseUrl: string, body: PairRequest): Promise<PairResult> {
  return request<PairResult>(baseUrl, "/api/clients/pair", { method: "POST", body: JSON.stringify(body) });
}

export function revokeClient(baseUrl: string, id: string): Promise<{ ok: boolean; id: string }> {
  return request(baseUrl, `/api/clients/${encodeURIComponent(id)}`, { method: "DELETE" });
}

export function getWsTicket(baseUrl: string): Promise<{ ticket: string; expires_in: number }> {
  return request(baseUrl, "/api/ws-ticket", { method: "POST" });
}
