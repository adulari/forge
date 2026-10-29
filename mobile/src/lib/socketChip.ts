import type { ConnectionState } from "./ws";

export type SocketChipTone = "ok" | "warn" | "muted" | "danger";

export interface SocketChipModel {
  label: string;
  tone: SocketChipTone;
}

/** What the session header's chip says about the socket, or null before the first connect. */
export function socketChipModel(state: ConnectionState, sessionEnded: boolean): SocketChipModel | null {
  if (sessionEnded) return { label: "Ended", tone: "muted" };
  switch (state) {
    case "open":
      return { label: "Live", tone: "ok" };
    case "connecting":
    case "reconnecting":
      return { label: "Reconnecting", tone: "warn" };
    case "paused":
      return { label: "Paused in background", tone: "muted" };
    case "unreachable":
      return { label: "Offline", tone: "danger" };
    case "closed":
      return { label: "Ended", tone: "muted" };
    default:
      return null;
  }
}
