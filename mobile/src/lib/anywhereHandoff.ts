import { idempotencyKey } from "./anywhereApi";

export type HandoffOutcome = "preparing" | "pending" | "accepted" | "failed" | "cancelled" | "indeterminate";

export interface CapsuleStatus {
  version: 1;
  capsule_id: string;
  state: "reserved" | "ready" | "claimed" | "acknowledged" | "failed" | "cancelled";
  acknowledgement_envelope: string | null;
  acknowledgement_signing_public_key: string | null;
}

export interface PendingCapsule {
  version: 1;
  capsule_id: string;
  source_host_id: string;
  source_device_id: string;
  key_epoch: number;
  sequence: number;
  ciphertext_bytes: number;
  ciphertext_sha256: string;
  expires_at_ms: number;
}

export interface PendingCapsules { version: 1; capsules: PendingCapsule[] }

export function handoffOutcome(status: CapsuleStatus | null, requestInFlight = false, requestFailed = false): HandoffOutcome {
  if (requestInFlight) return "preparing";
  if (requestFailed || status == null) return "indeterminate";
  if (["reserved", "ready", "claimed"].includes(status.state)) return "pending";
  if (status.state === "acknowledged") return "accepted";
  if (status.state === "failed") return "failed";
  return "cancelled";
}

export function handoffRecovery(outcome: HandoffOutcome): string {
  switch (outcome) {
    case "accepted": return "The destination acknowledged import and now owns the session lease.";
    case "failed": return "The source lease is unchanged. Resolve the reported destination conflict, then create a new handoff.";
    case "cancelled": return "The source lease is unchanged and the encrypted capsule is being removed.";
    case "indeterminate": return "Do not resume on both hosts. Refresh status before retrying; an accepted handoff may already own the lease.";
    case "preparing": return "Wait for the source host to reach an idle checkpoint and finish capsule upload.";
    case "pending": return "Keep the source paused while the destination verifies and imports the capsule.";
  }
}

export async function capsuleStatus(serviceUrl: string, token: string, capsuleId: string, fetcher: typeof fetch = fetch): Promise<CapsuleStatus> {
  validateId(capsuleId);
  return json<CapsuleStatus>(await fetcher(new URL(`/v1/capsules/${capsuleId}`, serviceUrl), { headers: { authorization: `Bearer ${token}`, accept: "application/json" }, cache: "no-store" }));
}

export async function pendingCapsules(serviceUrl: string, token: string, destinationHostId: string, fetcher: typeof fetch = fetch): Promise<PendingCapsules> {
  validateId(destinationHostId);
  const url = new URL("/v1/capsules", serviceUrl);
  url.searchParams.set("destination_host_id", destinationHostId);
  url.searchParams.set("state", "ready");
  return json<PendingCapsules>(await fetcher(url, { headers: { authorization: `Bearer ${token}`, accept: "application/json" }, cache: "no-store" }));
}

export async function cancelCapsule(serviceUrl: string, token: string, capsuleId: string, fetcher: typeof fetch = fetch): Promise<CapsuleStatus> {
  validateId(capsuleId);
  return json<CapsuleStatus>(await fetcher(new URL(`/v1/capsules/${capsuleId}`, serviceUrl), { method: "DELETE", headers: { authorization: `Bearer ${token}`, accept: "application/json", "Idempotency-Key": idempotencyKey() } }));
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) throw new Error(response.status === 404 ? "Handoff was not found or expired" : `Handoff request failed (${response.status})`);
  return await response.json() as T;
}
function validateId(value: string): void { if (!/^[0-9a-f]{32}$/.test(value)) throw new Error("Handoff id must be 32 lowercase hexadecimal characters"); }
