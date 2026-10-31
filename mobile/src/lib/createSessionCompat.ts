import type { CreateSessionRequest } from "./api";

function statusOf(error: unknown): number | null {
  return error instanceof Error && "status" in error && typeof error.status === "number" ? error.status : null;
}

/** A daemon older than `branch`/`effort` rejects the whole body (`deny_unknown_fields`). When
 * that is the failure, the same request without those two fields; otherwise null. */
export function withoutUnsupportedFields(body: CreateSessionRequest, error: unknown): CreateSessionRequest | null {
  const status = statusOf(error);
  if (status !== 400 && status !== 422) return null;
  const message = (error as Error).message;
  if (!message.includes("unknown field")) return null;
  if (!message.includes("`branch`") && !message.includes("`effort`")) return null;
  const { branch: _branch, effort: _effort, ...rest } = body;
  return rest;
}
