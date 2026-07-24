// Forge Anywhere context. Unlike queries.ts this is plain useState/useEffect over
// AnywhereClient (auth.tsx's pattern), not react-query — Anywhere account state is closer
// to "am I signed in" than to cacheable server data. Default is signed OUT: a fresh
// install (no persisted `forge.anywhere.account`) never probes the client and lands
// straight on the sign-in path, matching the design's "First use" application state.
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import { useAnywhere as useEncryptedAnywhere } from "../AnywhereProvider";
import { managedHostPresence } from "../anywhereHostPresence";
import { useAuth } from "../auth";
import type { AnywhereClient } from "./client";
import { deviceIdentityFingerprint, hostIdentityFingerprint, hostReachability } from "./hostIdentity";
import { MockAnywhereClient } from "./mockClient";
import type { AnywhereAccount, AnywhereDevice, AnywhereHost, RemoteJob, StorageInfo } from "./types";
import { daysUntilIso, normalizeEntitlementState } from "./format";

/** Module-level singleton mock backend — every consumer shares one in-memory instance. */
export const anywhereClient: AnywhereClient = new MockAnywhereClient();

interface AnywhereContextValue {
  account: AnywhereAccount | null;
  loading: boolean;
  client: AnywhereClient;
  signedIn: boolean;
  refresh: () => Promise<void>;
}

const AnywhereContext = createContext<AnywhereContextValue | null>(null);

export function AnywhereProvider({ children }: { children: React.ReactNode }) {
  const encrypted = useEncryptedAnywhere();
  const loading = encrypted.phase === "loading" || encrypted.phase === "starting";
  const account = useMemo<AnywhereAccount | null>(() => {
    if (encrypted.phase !== "ready" || !encrypted.credentials) return null;
    const quota = encrypted.account?.storage_limit_bytes ?? 0;
    const used = encrypted.account?.storage_used_bytes ?? 0;
    // Only the trial window has an unambiguous deadline on the wire (`trial_ends_at`, ISO).
    // Grace/read-only/suspended countdowns are deliberately left undefined rather than
    // guessed from `subscription`'s unit-less epoch numbers.
    const trialDaysLeft = daysUntilIso(encrypted.account?.trial_ends_at);
    return {
      githubLogin: encrypted.credentials.githubLogin ?? "signed-in account",
      entitlement: normalizeEntitlementState(encrypted.account?.entitlement),
      ...(trialDaysLeft === undefined ? {} : { trialDaysLeft }),
      relayConnected: true,
      lastSyncAt: null,
      storage: { usedBytes: used, quotaBytes: quota, state: quota > 0 && used >= quota ? "full" : "ok" },
    };
  }, [encrypted.account, encrypted.credentials, encrypted.phase]);

  const refresh = useCallback(async () => {
    await encrypted.refresh();
  }, [encrypted]);

  const value = useMemo<AnywhereContextValue>(
    () => ({ account, loading, client: anywhereClient, signedIn: account != null, refresh }),
    [account, loading, refresh],
  );

  return <AnywhereContext.Provider value={value}>{children}</AnywhereContext.Provider>;
}

export function useAnywhere(): AnywhereContextValue {
  const ctx = useContext(AnywhereContext);
  if (!ctx) throw new Error("useAnywhere must be used within AnywhereProvider");
  return ctx;
}

export function useAnywhereHosts() {
  const encrypted = useEncryptedAnywhere();
  const { servers } = useAuth();
  const signingPublicKeys = encrypted.credentials?.signingPublicKeys;
  const hosts = useMemo<AnywhereHost[]>(() => encrypted.hosts.map((host) => {
    const presence = managedHostPresence(host);
    const fingerprint = hostIdentityFingerprint(signingPublicKeys?.[host.device_id]);
    return {
      id: host.id,
      name: host.name,
      // Omitted rather than stubbed when unknown — the design type marks both optional.
      ...(fingerprint ? { fingerprint } : {}),
      ...(host.connector_version ? { connectorVersion: host.connector_version } : {}),
      heartbeatAgeSec: presence.heartbeatAgeSec,
      state: host.disabled === true ? { kind: "disabled" } : presence.state,
      reachableVia: hostReachability(servers, host.name),
      transportPreference: encrypted.hostTransportPreferences[host.id] ?? "auto",
    };
  }), [encrypted.hostTransportPreferences, encrypted.hosts, servers, signingPublicKeys]);
  return { hosts, loading: encrypted.phase === "loading", refresh: encrypted.refresh };
}

export function useAnywhereDevices() {
  const encrypted = useEncryptedAnywhere();
  const devices = useMemo<AnywhereDevice[]>(() => encrypted.devices.map((device) => {
    const fingerprint = deviceIdentityFingerprint(device.signing_public_key);
    return {
      id: device.id,
      name: device.name,
      kind: /phone|iphone|android/i.test(device.name) ? "phone" : "laptop",
      ...(fingerprint ? { fingerprint } : {}),
      enrolledAt: Date.parse(device.created_at),
      lastSeenAt: device.last_seen_at ? Date.parse(device.last_seen_at) : Date.parse(device.created_at),
      isThisDevice: device.id === encrypted.credentials?.deviceIdHex,
    } satisfies AnywhereDevice;
  }), [encrypted.credentials?.deviceIdHex, encrypted.devices]);
  return { devices, loading: encrypted.phase === "loading", refresh: encrypted.refresh };
}

export function useAnywhereJobs() {
  const { client, signedIn } = useAnywhere();
  const [jobs, setJobs] = useState<RemoteJob[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setJobs([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setJobs(await client.listJobs());
    } finally {
      setLoading(false);
    }
  }, [client, signedIn]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { jobs, loading, refresh };
}

export function useAnywhereStorage() {
  const encrypted = useEncryptedAnywhere();
  const quota = encrypted.account?.storage_limit_bytes ?? 0;
  const used = encrypted.account?.storage_used_bytes ?? 0;
  const storage: StorageInfo | null = encrypted.phase === "ready"
    ? { usedBytes: used, quotaBytes: quota, state: quota > 0 && used >= quota ? "full" : "ok" }
    : null;
  return { storage, loading: encrypted.phase === "loading", refresh: encrypted.refresh };
}
