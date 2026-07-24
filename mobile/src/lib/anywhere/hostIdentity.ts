// Identity and reachability facts a managed host can state truthfully.
//
// Nothing here invents data. The fingerprint is the SHA256 of the host device's own
// enrolled Ed25519 signing key — the same key material the pairing safety code is built
// from — so it is stable across renames and reconnects, exactly as the comp claims, and
// it is `null` whenever that key is not known to this device. Reachability is derived
// from the transports this device actually holds: every enrolled host has a registered
// relay transport (AnywhereProvider registers one per host), and a direct/LAN leg exists
// only when a saved direct server target reports the same daemon hostname.
import { sha256 } from "@noble/hashes/sha2.js";

import { fromBase64Url } from "../anywhereApi";
import type { StoredServer } from "../serverTargets";
import { bytesFromHex, bytesToHex } from "../transport/anywhereEnvelope";
import type { HostReachability } from "./types";

/** `SHA256:4b7d…e910` — first and last two bytes of the digest, as in the design comp. */
function fingerprintOf(decode: () => Uint8Array): string | null {
  let digest: string;
  try {
    digest = bytesToHex(sha256(decode()));
  } catch {
    return null;
  }
  return `SHA256:${digest.slice(0, 4)}…${digest.slice(-4)}`;
}

/** Host identity, from the hex signing key cached in the credential store. */
export function hostIdentityFingerprint(signingPublicKeyHex: string | undefined | null): string | null {
  if (!signingPublicKeyHex) return null;
  return fingerprintOf(() => bytesFromHex(signingPublicKeyHex));
}

/** Device identity, from the base64url signing key `GET /v1/devices` returns. */
export function deviceIdentityFingerprint(signingPublicKeyBase64Url: string | undefined | null): string | null {
  if (!signingPublicKeyBase64Url) return null;
  return fingerprintOf(() => fromBase64Url(signingPublicKeyBase64Url));
}

/** Hostnames compare case-insensitively and ignore an mDNS `.local` suffix. */
function normalizedHostname(value: string): string {
  return value.trim().toLowerCase().replace(/\.local\.?$/, "");
}

/**
 * The saved direct/LAN target for a managed host, matched on the daemon hostname
 * `GET /api/identity` reports (`StoredServer.name` once identity has been applied) or on
 * the connect URL's host. A renamed Anywhere host stops matching until the daemon is
 * renamed too — the match is a hostname claim, never an identity claim.
 */
export function directServerForHost(
  servers: readonly StoredServer[],
  hostName: string,
): StoredServer | null {
  const wanted = normalizedHostname(hostName);
  if (!wanted) return null;
  return servers.find((server) => server.transport !== "anywhere"
    && (normalizedHostname(server.name) === wanted || normalizedHostname(server.host) === wanted)) ?? null;
}

export function hostReachability(
  servers: readonly StoredServer[],
  hostName: string,
): HostReachability[] {
  const reachable: HostReachability[] = [];
  if (directServerForHost(servers, hostName)) reachable.push("direct-lan");
  reachable.push("anywhere-relay");
  return reachable;
}

const REACHABILITY_LABEL: Record<HostReachability, string> = {
  "direct-lan": "direct · lan",
  "anywhere-relay": "anywhere · relay",
};

/** `direct · lan and anywhere · relay` — the comp's phrasing. */
export function formatReachability(reachable: readonly HostReachability[]): string {
  if (!reachable.length) return "no transport";
  return reachable.map((kind) => REACHABILITY_LABEL[kind]).join(" and ");
}
