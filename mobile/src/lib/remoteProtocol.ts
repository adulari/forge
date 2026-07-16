export const PROTOCOL_VERSION = 8;

export interface SnapshotIdentity {
  protocol: number;
  session_id: string;
  revision: number;
  resync: boolean;
  closed: boolean;
}

/** Minimal structural guard for untrusted WebSocket frames. */
export function isValidSnapshotFrame(value: unknown): value is SnapshotIdentity {
  if (value == null || typeof value !== "object") return false;
  const frame = value as Record<string, unknown>;
  return (
    typeof frame.protocol === "number" &&
    typeof frame.session_id === "string" &&
    typeof frame.revision === "number" &&
    Number.isFinite(frame.revision) &&
    typeof frame.resync === "boolean" &&
    typeof frame.closed === "boolean"
  );
}
