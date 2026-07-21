import type { ConnectTestState } from "./auth";

interface FleetQueryHealth {
  isSuccess: boolean;
  isLoading: boolean;
  error: unknown;
}

/** Keep Settings' health row on the same live query that colors its active server row. */
export function connectionHealthFromFleet(query?: FleetQueryHealth): ConnectTestState {
  if (!query) return "idle";
  if (query.isSuccess) return "ok";
  if (query.isLoading) return "testing";
  const status = (query.error as { status?: number } | null)?.status;
  if (status === 404) return "bad-token";
  if (status === 0) return "unreachable";
  return "server-error";
}

export type DesktopFleetStatus = {
  state: "online" | "loading" | "offline" | "error";
  label: "online" | "connecting" | "offline" | "service unavailable";
};

export function desktopFleetStatusFromFleet(query: FleetQueryHealth): DesktopFleetStatus {
  const health = connectionHealthFromFleet(query);
  if (health === "ok") return { state: "online", label: "online" };
  if (health === "testing" || health === "idle") return { state: "loading", label: "connecting" };
  if (health === "unreachable") return { state: "offline", label: "offline" };
  return { state: "error", label: "service unavailable" };
}
