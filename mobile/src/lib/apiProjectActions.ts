// Project actions (`[[actions]]` in .forge/config.toml) and the ports a session listens on, over
// `/api/terminals/actions/*` and `/api/ports` (serve_project_actions.rs, serve_ports.rs).
import { request } from "./api";
import {
  parsePorts,
  parseProjectActions,
  parseRunningActions,
} from "./projectActionsModel";

export * from "./projectActionsModel";

function query(params: Record<string, string>): string {
  return `?${Object.entries(params)
    .map(([key, value]) => `${encodeURIComponent(key)}=${encodeURIComponent(value)}`)
    .join("&")}`;
}

export const getProjectActions = async (baseUrl: string, session: string) =>
  parseProjectActions(await request<unknown>(baseUrl, `/api/terminals/actions${query({ session })}`));

export const getRunningActions = async (baseUrl: string) =>
  parseRunningActions(await request<unknown>(baseUrl, "/api/terminals/actions/running"));

export const runProjectAction = (baseUrl: string, session: string, action: string) =>
  request<{ ok: boolean; terminal_id: string }>(baseUrl, "/api/terminals/actions/run", {
    method: "POST",
    body: JSON.stringify({ session, action }),
  });

export const stopProjectAction = (baseUrl: string, session: string, action: string) =>
  request<{ ok: boolean; stopped: boolean }>(baseUrl, "/api/terminals/actions/stop", {
    method: "POST",
    body: JSON.stringify({ session, action }),
  });

export const getSessionPorts = async (baseUrl: string, session: string) =>
  parsePorts(await request<unknown>(baseUrl, `/api/ports${query({ session })}`));
