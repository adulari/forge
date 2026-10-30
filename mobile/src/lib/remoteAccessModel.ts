import type { AccessAddress, AccessScope, PairedClient, WhoAmI } from "./apiAccess";

export const ADDRESS_LABELS: Record<AccessAddress["kind"], string> = {
  local: "Local",
  lan: "LAN",
  tailscale: "Tailscale",
  public: "Public",
};

export const SCOPE_LABELS: Record<AccessScope, string> = {
  view: "View",
  prompt: "Prompt",
  approve: "Approve",
  terminal: "Terminal",
  "workspace-write": "Edit files",
  "manage-providers": "Providers",
};

export interface ConnectionAccess {
  loaded: boolean;
  owner: boolean;
  canPrompt: boolean;
  canApprove: boolean;
  viewOnly: boolean;
}

/** A host that predates `/api/whoami`, or an Anywhere connection, is treated as full access. */
export const FULL_ACCESS: ConnectionAccess = { loaded: true, owner: true, canPrompt: true, canApprove: true, viewOnly: false };

export function accessFrom(me: WhoAmI | null | undefined): ConnectionAccess {
  if (!me) return FULL_ACCESS;
  const canPrompt = me.scopes.includes("prompt");
  const canApprove = me.scopes.includes("approve");
  return { loaded: true, owner: me.owner, canPrompt, canApprove, viewOnly: !canPrompt };
}

export function pairingLink(address: AccessAddress | undefined, token: string | null): string | null {
  if (!address?.base || !token) return null;
  return `${address.base.replace(/\/+$/, "")}/${token}`;
}

export function lastConnectedLabel(client: PairedClient, nowMs: number): string {
  if (client.pending) return "Waiting to pair";
  if (client.last_seen == null) return "Never connected";
  const secs = Math.max(0, Math.round(nowMs / 1000 - client.last_seen));
  if (secs < 60) return "Just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86_400)}d ago`;
}

export function scopeSummary(client: PairedClient): string {
  if (client.preset === "viewer") return "View only";
  if (client.scopes.length >= 6) return "Full access";
  return client.scopes.map((s) => SCOPE_LABELS[s]).join(", ");
}

/** First address that can be reached, so the QR defaults to something that works. */
export function defaultAddressKind(addresses: AccessAddress[], mode: string): AccessAddress["kind"] {
  const preferred = mode === "anywhere" ? "public" : mode === "lan" ? "lan" : "local";
  return addresses.find((a) => a.kind === preferred && a.base)?.kind ?? addresses.find((a) => a.base)?.kind ?? "local";
}

/** Seconds after which a shown pairing code is replaced, a little before the host drops it. */
export function rotateAfterSeconds(expiresIn: number): number {
  return Math.max(15, expiresIn - 30);
}
