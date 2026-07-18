// Forge Anywhere context. Unlike queries.ts this is plain useState/useEffect over
// AnywhereClient (auth.tsx's pattern), not react-query — Anywhere account state is closer
// to "am I signed in" than to cacheable server data. Default is signed OUT: a fresh
// install (no persisted `forge.anywhere.account`) never probes the client and lands
// straight on the sign-in path, matching the design's "First use" application state.
import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

import type { AnywhereClient } from "./client";
import { MockAnywhereClient } from "./mockClient";
import type { AnywhereAccount, AnywhereDevice, AnywhereHost, RemoteJob, StorageInfo } from "./types";

const ACCOUNT_KEY = "forge.anywhere.account";

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
  const [account, setAccount] = useState<AnywhereAccount | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    const next = await anywhereClient.getAccount();
    setAccount(next);
    if (next) {
      await AsyncStorage.setItem(ACCOUNT_KEY, JSON.stringify({ githubLogin: next.githubLogin }));
    } else {
      await AsyncStorage.removeItem(ACCOUNT_KEY);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const stored = await AsyncStorage.getItem(ACCOUNT_KEY);
        if (stored) {
          const next = await anywhereClient.getAccount();
          if (!cancelled) setAccount(next);
        }
      } catch {
        // Fail open to signed-out — same posture as auth.tsx's boot-load failure handling.
        if (!cancelled) setAccount(null);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

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
  const { client, signedIn } = useAnywhere();
  const [hosts, setHosts] = useState<AnywhereHost[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setHosts([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setHosts(await client.listHosts());
    } finally {
      setLoading(false);
    }
  }, [client, signedIn]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { hosts, loading, refresh };
}

export function useAnywhereDevices() {
  const { client, signedIn } = useAnywhere();
  const [devices, setDevices] = useState<AnywhereDevice[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setDevices([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setDevices(await client.listDevices());
    } finally {
      setLoading(false);
    }
  }, [client, signedIn]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { devices, loading, refresh };
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
  const { client, signedIn } = useAnywhere();
  const [storage, setStorage] = useState<StorageInfo | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    if (!signedIn) {
      setStorage(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      setStorage(await client.getStorage());
    } finally {
      setLoading(false);
    }
  }, [client, signedIn]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { storage, loading, refresh };
}
