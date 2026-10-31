// The control toolset over `/api/control` (serve_control.rs): which tools an agent may use to
// drive the app, and the audit trail of what it did. Direct daemon connections only.
import { request } from "./api";

export interface ControlTool {
  name: string;
  description: string;
  kind: "read" | "act";
  enabled: boolean;
}

export interface ControlAuditEntry {
  ts: string;
  route: string;
  decision: string;
  session: string | null;
  detail: string | null;
}

export interface ControlOverview {
  tools: ControlTool[];
  audit: ControlAuditEntry[];
}

export function getControl(baseUrl: string): Promise<ControlOverview> {
  return request<ControlOverview>(baseUrl, "/api/control");
}

export function setControlTool(baseUrl: string, name: string, enabled: boolean): Promise<{ tools: ControlTool[] }> {
  return request(baseUrl, `/api/control/tools/${encodeURIComponent(name)}`, {
    method: "POST",
    body: JSON.stringify({ enabled }),
  });
}

/** `control/create_session` -> `create_session`. */
export function auditTool(entry: ControlAuditEntry): string {
  return entry.route.replace(/^control\//, "");
}
