import * as Linking from "expo-linking";
import { Platform } from "react-native";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";

import {
  DEFAULT_ANYWHERE_SERVICE_URL,
  AnywhereApiError,
  type AnywhereAccountStatus,
  type AnywhereAuthSession,
  type AnywhereBillingPeriod,
  type AnywhereCheckoutSession,
  type AnywhereDevice,
  type AnywhereDeviceFlow,
  type AnywhereHost,
  type AnywherePortalSession,
  type AnywhereRecoveryWrap,
  type AnywhereSubscription,
  anywhereRequest,
  base64Url,
  fromBase64Url,
  idempotencyKey,
} from "./anywhereApi";
import {
  billingCheckoutBody,
  commitPendingDeviceRevocation,
  prepareDeviceRevocation,
  refreshAnywhereCredentials,
  stagePreparedDeviceRevocation,
} from "./anywhereAccountOperations";
import {
  deriveRecoveryWrapKey,
  deriveSelfDeviceWrapKey,
  generatePendingKeys,
  generateRecoveryPhrase,
  makeKeyWrap,
  openRecoveryWrap,
  type PendingAnywhereKeys,
} from "./anywhereCrypto";
import { clearAnywhereHostCache, readAnywhereHostCache, writeAnywhereHostCache } from "./anywhereHostCache";
import { acceptReplaySequences } from "./anywhereReplayWindow";
import {
  clearAnywherePushState,
  disableAnywherePush,
  enableAnywherePush,
  getAnywherePushStatus,
  observeAnywherePushRefresh,
  type AnywherePushStatus,
} from "./anywherePush";
import type { AnywherePushApi } from "./anywherePushCore";
import { useAuth } from "./auth";
import {
  AnywhereTransport,
  EncryptedAnywhereRelay,
  anywhereCredentialStore,
  registerAnywhereTransport,
  type AnywhereRelayCredentials,
  type StoredAnywhereCredentials,
} from "./transport";
import { bytesFromHex, bytesToHex } from "./transport/anywhereEnvelope";

const SERVICE_URL = process.env.EXPO_PUBLIC_FORGE_ANYWHERE_URL ?? DEFAULT_ANYWHERE_SERVICE_URL;
const INITIAL_EPOCH = 1;

type PendingLogin = { auth: AnywhereAuthSession; keys: PendingAnywhereKeys };
type RecoverySetup = PendingLogin & { words: string; entropy: Uint8Array; dataKey: Uint8Array };

export type AnywherePhase =
  | "loading"
  | "signed_out"
  | "starting"
  | "authorizing"
  | "new_recovery"
  | "existing_recovery"
  | "ready"
  | "error";

interface AnywhereContextValue {
  phase: AnywherePhase;
  credentials: StoredAnywhereCredentials | null;
  account: AnywhereAccountStatus | null;
  subscription: AnywhereSubscription | null;
  hosts: AnywhereHost[];
  devices: AnywhereDevice[];
  flow: AnywhereDeviceFlow | null;
  recoveryWords: string | null;
  recoverySample: readonly number[];
  error: string | null;
  pushStatus: AnywherePushStatus;
  /** Returns a fresh short-lived token for first-party Anywhere clients; never persist it. */
  accessToken(): Promise<string>;
  startLogin(): Promise<void>;
  confirmNewRecovery(answers: Record<number, string>): Promise<void>;
  recoverExisting(words: string): Promise<void>;
  refresh(): Promise<void>;
  checkout(period?: AnywhereBillingPeriod): Promise<void>;
  openBillingPortal(): Promise<void>;
  revokeDevice(deviceId: string, recoveryWords: string): Promise<void>;
  revokeHost(hostId: string): Promise<void>;
  selectHost(hostId: string): void;
  enablePush(): Promise<void>;
  disablePush(): Promise<void>;
  logout(): Promise<void>;
}

const AnywhereContext = createContext<AnywhereContextValue | null>(null);
const credentialStore = anywhereCredentialStore();

export function AnywhereProvider({ children }: { children: React.ReactNode }) {
  const auth = useAuth();
  const { setActive, syncAnywhereHosts } = auth;
  const [phase, setPhase] = useState<AnywherePhase>("loading");
  const [credentials, setCredentials] = useState<StoredAnywhereCredentials | null>(null);
  const credentialsRef = useRef<StoredAnywhereCredentials | null>(null);
  const [account, setAccount] = useState<AnywhereAccountStatus | null>(null);
  const [subscription, setSubscription] = useState<AnywhereSubscription | null>(null);
  const [hosts, setHosts] = useState<AnywhereHost[]>([]);
  const [devices, setDevices] = useState<AnywhereDevice[]>([]);
  const [flow, setFlow] = useState<AnywhereDeviceFlow | null>(null);
  const [pending, setPending] = useState<PendingLogin | null>(null);
  const [recoverySetup, setRecoverySetup] = useState<RecoverySetup | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pushStatus, setPushStatus] = useState<AnywherePushStatus>("unsubscribed");
  const mutationQueue = useRef(Promise.resolve());
  const revocationRecoveryAttempt = useRef<string | null>(null);

  const persistCredentials = useCallback(async (next: StoredAnywhereCredentials) => {
    credentialsRef.current = next;
    await credentialStore.save(next);
    setCredentials(next);
  }, []);

  const accessToken = useCallback(async (): Promise<string> => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in");
    if (current.accessExpiresAtMs > Date.now() + 30_000) return current.accessToken;
    let resolved = "";
    mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
      const latest = credentialsRef.current;
      if (!latest) throw new Error("Forge Anywhere is not signed in");
      const next = await refreshAnywhereCredentials(
        latest,
        (refreshToken) => anywhereRequest(latest.serviceUrl ?? SERVICE_URL, "/v1/auth/refresh", {
          method: "POST",
          body: JSON.stringify({ refresh_token: refreshToken }),
        }),
        persistCredentials,
      );
      resolved = next.accessToken;
    });
    await mutationQueue.current;
    return resolved;
  }, [persistCredentials]);

  const pushApi = useMemo<AnywherePushApi>(() => ({
    register: async (input) => {
      const token = await accessToken();
      const serviceUrl = credentialsRef.current?.serviceUrl ?? SERVICE_URL;
      return anywhereRequest<{ subscription_id: string }>(serviceUrl, "/v1/push/subscriptions", {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify(input),
      }, token);
    },
    revoke: async (subscriptionId) => {
      const token = await accessToken();
      const serviceUrl = credentialsRef.current?.serviceUrl ?? SERVICE_URL;
      await anywhereRequest(serviceUrl, `/v1/push/subscriptions/${subscriptionId}/revoke`, {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
      }, token);
    },
  }), [accessToken]);

  /* eslint-disable react-hooks/refs -- these relay callbacks read refs only when invoked after render */
  const runtime = useMemo<AnywhereRelayCredentials | null>(() => {
    if (!credentials) return null;
    return {
      serviceUrl: credentials.serviceUrl ?? SERVICE_URL,
      accountId: bytesFromHex(credentials.accountIdHex),
      deviceId: bytesFromHex(credentials.deviceIdHex),
      dataKey: bytesFromHex(credentials.accountDataKeyHex),
      keyEpoch: credentials.keyEpoch,
      signingPrivateKey: bytesFromHex(credentials.signingPrivateKeyHex),
      accessToken,
      reserveSequence: async () => {
        let reserved = 0n;
        mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
          const latest = credentialsRef.current;
          if (!latest) throw new Error("Forge Anywhere is not signed in");
          reserved = BigInt(latest.nextSequence);
          await persistCredentials({ ...latest, nextSequence: (reserved + 1n).toString() });
        });
        await mutationQueue.current;
        return reserved;
      },
      acceptSequences: async (sender, epoch, sequences) => {
        let accepted = false;
        mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
          const latest = credentialsRef.current;
          if (!latest) return;
          const namespace = `${sender}:${epoch}`;
          const result = acceptReplaySequences(latest.acceptedSequences[namespace], sequences);
          if (!result.accepted) return;
          const next = { ...latest, acceptedSequences: { ...latest.acceptedSequences, [namespace]: result.next } };
          await persistCredentials(next);
          accepted = true;
        });
        await mutationQueue.current;
        return accepted;
      },
      signingPublicKey: async (sender) => {
        const value = credentialsRef.current?.signingPublicKeys[sender];
        if (!value) throw new Error("Forge Anywhere sender is not an enrolled device");
        return bytesFromHex(value);
      },
      randomBytes: (length) => crypto.getRandomValues(new Uint8Array(length)),
    };
    // Runtime remains stable while tokens/sequences mutate; a key/device epoch change replaces it.
    // Only key identity changes replace live relays; token and replay-state writes are read via refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accessToken, credentials?.accountDataKeyHex, credentials?.accountIdHex, credentials?.deviceIdHex, credentials?.keyEpoch, credentials?.serviceUrl, credentials?.signingPrivateKeyHex]);
  /* eslint-enable react-hooks/refs */

  const refresh = useCallback(async () => {
    if (!credentialsRef.current) return;
    try {
      const token = await accessToken();
      const serviceUrl = credentialsRef.current?.serviceUrl ?? SERVICE_URL;
      const [nextAccount, nextSubscription, hostList, deviceList] = await Promise.all([
        anywhereRequest<AnywhereAccountStatus>(serviceUrl, "/v1/me", {}, token),
        anywhereRequest<AnywhereSubscription>(serviceUrl, "/v1/billing/subscription", {}, token),
        anywhereRequest<{ hosts: AnywhereHost[] }>(serviceUrl, "/v1/hosts", {}, token),
        anywhereRequest<{ devices: AnywhereDevice[] }>(serviceUrl, "/v1/devices", {}, token),
      ]);
      const publicKeys = Object.fromEntries(deviceList.devices.map((device) => [device.id, bytesToHex(fromBase64Url(device.signing_public_key))]));
      const latest = credentialsRef.current;
      if (latest && JSON.stringify(latest.signingPublicKeys) !== JSON.stringify(publicKeys)) {
        await persistCredentials({ ...latest, signingPublicKeys: publicKeys });
      }
      setAccount(nextAccount);
      setSubscription(nextSubscription);
      setHosts(hostList.hosts);
      setDevices(deviceList.devices);
      if (latest) void writeAnywhereHostCache(latest.accountIdHex, hostList.hosts).catch(() => {
        // The live fleet remains usable if the optional metadata cache is unavailable.
      });
      setError(null);
      setPhase("ready");
    } catch (reason) {
      setError(message(reason));
      // An account-status refresh must not tear down already registered encrypted transports.
      setPhase(credentialsRef.current ? "ready" : "error");
    }
  }, [accessToken, persistCredentials]);

  useEffect(() => {
    let cancelled = false;
    void credentialStore.load().then(async (loaded) => {
      if (cancelled) return;
      const cachedHosts = loaded ? await readAnywhereHostCache(loaded.accountIdHex) : [];
      if (cancelled) return;
      credentialsRef.current = loaded;
      setCredentials(loaded);
      if (loaded) setHosts(cachedHosts);
      setPhase(loaded ? "ready" : "signed_out");
      if (loaded) void refresh();
    }).catch((reason) => {
      if (!cancelled) { setError(message(reason)); setPhase("error"); }
    });
    return () => { cancelled = true; };
  }, [refresh]);

  const pushDeviceId = credentials?.deviceIdHex;
  useEffect(() => {
    if (phase !== "ready" || !pushDeviceId) {
      setPushStatus("unsubscribed");
      return;
    }
    let cancelled = false;
    void getAnywherePushStatus().then((status) => {
      if (!cancelled) setPushStatus(status);
    }).catch(() => {
      if (!cancelled) setPushStatus("unsubscribed");
    });
    const stop = observeAnywherePushRefresh(() => { void refresh(); });
    return () => { cancelled = true; stop(); };
  }, [phase, pushDeviceId, refresh]);

  useEffect(() => {
    const pendingRevocation = credentials?.pendingDeviceRevocation;
    if (!pendingRevocation || revocationRecoveryAttempt.current === pendingRevocation.idempotencyKey) return;
    revocationRecoveryAttempt.current = pendingRevocation.idempotencyKey;
    void accessToken().then(async (token) => {
      mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
        const latest = credentialsRef.current;
        if (!latest?.pendingDeviceRevocation) return;
        const pending = latest.pendingDeviceRevocation;
        await commitPendingDeviceRevocation(
          pending,
          (request, key) => anywhereRequest(latest.serviceUrl ?? SERVICE_URL, `/v1/devices/${pending.targetDeviceId}/revoke`, {
            method: "POST", headers: { "Idempotency-Key": key }, body: JSON.stringify(request),
          }, token),
          async () => deviceRevocationCommitted(latest.serviceUrl ?? SERVICE_URL, token, pending),
          persistCredentials,
        );
      });
      await mutationQueue.current;
      await refresh();
    }).catch((reason) => {
      revocationRecoveryAttempt.current = null;
      setError(message(reason));
    });
  }, [accessToken, credentials?.pendingDeviceRevocation, persistCredentials, refresh]);

  useEffect(() => {
    if (!runtime || phase !== "ready" || auth.isLoading) return;
    const removers = hosts.map((host) => registerAnywhereTransport(
      new AnywhereTransport(host.id, new EncryptedAnywhereRelay(runtime)),
    ));
    void syncAnywhereHosts(hosts.map((host) => ({ id: host.id, name: host.name })));
    return () => { for (const remove of removers) remove(); };
  }, [auth.isLoading, hosts, phase, runtime, syncAnywhereHosts]);

  const startLogin = useCallback(async () => {
    setError(null);
    setPhase("starting");
    try {
      const keys = generatePendingKeys();
      const started = await anywhereRequest<AnywhereDeviceFlow>(SERVICE_URL, "/v1/auth/github/start", {
        method: "POST",
        body: JSON.stringify({
          device_name: Platform.OS === "web" ? "Forge web" : `Forge ${Platform.OS}`,
          signing_public_key: base64Url(keys.signingPublicKey),
          exchange_public_key: base64Url(keys.exchangePublicKey),
        }),
      });
      setPending({ auth: null as never, keys });
      setFlow(started);
      setPhase("authorizing");
      void Linking.openURL(started.verification_uri).catch(() => {
        // The code and a retryable link remain visible if a browser blocks the automatic open.
      });
    } catch (reason) {
      setError(message(reason));
      setPhase("error");
    }
  }, []);

  useEffect(() => {
    if (phase !== "authorizing" || !flow || !pending) return;
    let cancelled = false;
    const deadline = Date.now() + flow.expires_in * 1000;
    const timer = setInterval(() => {
      if (cancelled) return;
      if (Date.now() >= deadline) {
        clearInterval(timer);
        setError("GitHub login expired. Start again to receive a new code.");
        setPhase("error");
        return;
      }
      void anywhereRequest<AnywhereAuthSession | undefined>(SERVICE_URL, "/v1/auth/device/poll", {
        method: "POST", body: JSON.stringify({ device_code: flow.device_code }),
      }).then((session) => {
        if (cancelled || !session) return;
        clearInterval(timer);
        const nextPending = { auth: session, keys: pending.keys };
        setPending(nextPending);
        if (session.new_account) {
          const recovery = generateRecoveryPhrase();
          setRecoverySetup({ ...nextPending, ...recovery, dataKey: crypto.getRandomValues(new Uint8Array(32)) });
          setPhase("new_recovery");
        } else {
          setPhase("existing_recovery");
        }
      }).catch((reason) => {
        if (reason instanceof AnywhereApiError && reason.status === 202) return;
        if (!cancelled) { clearInterval(timer); setError(message(reason)); setPhase("error"); }
      });
    }, Math.max(1, flow.interval) * 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [flow, pending, phase]);

  const finishEnrollment = useCallback(async (
    login: PendingLogin,
    dataKey: Uint8Array,
    epoch: number,
    nextSequence: bigint,
  ) => {
    const next: StoredAnywhereCredentials = {
      version: 1,
      serviceUrl: SERVICE_URL,
      githubLogin: login.auth.github_login,
      accountIdHex: login.auth.account_id,
      deviceIdHex: login.auth.device_id,
      signingPrivateKeyHex: bytesToHex(login.keys.signingPrivateKey),
      exchangePrivateKeyHex: bytesToHex(login.keys.exchangePrivateKey),
      accountDataKeyHex: bytesToHex(dataKey),
      dataKeyEpochs: { [String(epoch)]: bytesToHex(dataKey) },
      keyEpoch: epoch,
      accessToken: login.auth.access_token,
      refreshToken: login.auth.refresh_token,
      accessExpiresAtMs: login.auth.access_expires_at_ms,
      nextSequence: nextSequence.toString(),
      acceptedSequences: {},
      signingPublicKeys: { [login.auth.device_id]: bytesToHex(login.keys.signingPublicKey) },
    };
    await persistCredentials(next);
    setPending(null);
    setRecoverySetup(null);
    setFlow(null);
    setPhase("ready");
    await refresh();
  }, [persistCredentials, refresh]);

  const confirmNewRecovery = useCallback(async (answers: Record<number, string>) => {
    if (!recoverySetup) return;
    const words = recoverySetup.words.split(" ");
    const samples = [3, 11, 20];
    if (samples.some((index) => answers[index]?.trim().toLowerCase() !== words[index])) {
      setError("Those recovery words do not match. Check the numbered words and try again.");
      return;
    }
    setError(null);
    const accountId = bytesFromHex(recoverySetup.auth.account_id);
    const deviceId = bytesFromHex(recoverySetup.auth.device_id);
    const deviceKey = deriveSelfDeviceWrapKey(recoverySetup.keys.exchangePrivateKey, recoverySetup.keys.exchangePublicKey, accountId, INITIAL_EPOCH);
    const recoveryKey = deriveRecoveryWrapKey(recoverySetup.entropy, accountId, INITIAL_EPOCH);
    const deviceWrap = makeKeyWrap(recoverySetup.dataKey, deviceKey, accountId, deviceId, 1, deviceId, INITIAL_EPOCH, 0n, recoverySetup.keys.signingPrivateKey);
    const recoveryWrap = makeKeyWrap(recoverySetup.dataKey, recoveryKey, accountId, deviceId, 3, accountId, INITIAL_EPOCH, 1n, recoverySetup.keys.signingPrivateKey);
    try {
      await anywhereRequest(SERVICE_URL, "/v1/key-epochs", {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ epoch: INITIAL_EPOCH, device_wrap_envelope: base64Url(deviceWrap), recovery_wrap_envelope: base64Url(recoveryWrap) }),
      }, recoverySetup.auth.access_token);
      await finishEnrollment(recoverySetup, recoverySetup.dataKey, INITIAL_EPOCH, 2n);
    } catch (reason) { setError(message(reason)); }
  }, [finishEnrollment, recoverySetup]);

  const recoverExisting = useCallback(async (words: string) => {
    if (!pending?.auth.recovery_wrap_envelope || !pending.auth.recovery_wrap_signing_public_key) {
      setError("The service did not return this account's encrypted recovery key.");
      return;
    }
    try {
      setError(null);
      const recovered = openRecoveryWrap(pending.auth.recovery_wrap_envelope, pending.auth.recovery_wrap_signing_public_key, words, pending.auth.account_id);
      const accountId = bytesFromHex(pending.auth.account_id);
      const deviceId = bytesFromHex(pending.auth.device_id);
      const deviceKey = deriveSelfDeviceWrapKey(pending.keys.exchangePrivateKey, pending.keys.exchangePublicKey, accountId, recovered.epoch);
      const wrap = makeKeyWrap(recovered.dataKey, deviceKey, accountId, deviceId, 1, deviceId, recovered.epoch, 0n, pending.keys.signingPrivateKey);
      await anywhereRequest(SERVICE_URL, `/v1/key-epochs/${recovered.epoch}/wraps`, {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ epoch: recovered.epoch, device_wrap_envelope: base64Url(wrap) }),
      }, pending.auth.access_token);
      await finishEnrollment(pending, recovered.dataKey, recovered.epoch, 1n);
    } catch (reason) { setError(message(reason)); }
  }, [finishEnrollment, pending]);

  const selectHost = useCallback((hostId: string) => setActive(`anywhere:${hostId}`), [setActive]);

  const checkout = useCallback(async (period?: AnywhereBillingPeriod) => {
    try {
      setError(null);
      const token = await accessToken();
      const current = credentialsRef.current;
      if (!current) throw new Error("Forge Anywhere is not signed in");
      const session = await anywhereRequest<AnywhereCheckoutSession>(
        current.serviceUrl ?? SERVICE_URL,
        "/v1/billing/checkout",
        {
          method: "POST",
          headers: { "Idempotency-Key": idempotencyKey() },
          body: JSON.stringify(billingCheckoutBody(period)),
        },
        token,
      );
      await Linking.openURL(session.checkout_url);
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [accessToken]);

  const openBillingPortal = useCallback(async () => {
    try {
      setError(null);
      const token = await accessToken();
      const current = credentialsRef.current;
      if (!current) throw new Error("Forge Anywhere is not signed in");
      const session = await anywhereRequest<AnywherePortalSession>(
        current.serviceUrl ?? SERVICE_URL,
        "/v1/billing/portal",
        { method: "POST", headers: { "Idempotency-Key": idempotencyKey() } },
        token,
      );
      await Linking.openURL(session.portal_url);
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [accessToken]);

  const revokeHost = useCallback(async (hostId: string) => {
    try {
      setError(null);
      const token = await accessToken();
      const current = credentialsRef.current;
      if (!current) throw new Error("Forge Anywhere is not signed in");
      await anywhereRequest(current.serviceUrl ?? SERVICE_URL, `/v1/hosts/${hostId}`, {
        method: "DELETE",
        headers: { "Idempotency-Key": idempotencyKey() },
      }, token);
      await refresh();
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [accessToken, refresh]);

  const revokeDevice = useCallback(async (deviceId: string, recoveryWords: string) => {
    try {
      setError(null);
      const token = await accessToken();
      const current = credentialsRef.current;
      if (!current) throw new Error("Forge Anywhere is not signed in");
      const recovery = await anywhereRequest<AnywhereRecoveryWrap>(
        current.serviceUrl ?? SERVICE_URL,
        `/v1/key-epochs/${current.keyEpoch}/wraps/recovery`,
        {},
        token,
      );
      // Sequence reservation and promotion share the relay mutation queue. Relay frames cannot
      // claim a wrap sequence while revocation is in flight or overwrite the promoted epoch.
      mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
        const latest = credentialsRef.current;
        if (!latest) throw new Error("Forge Anywhere is not signed in");
        const prepared = prepareDeviceRevocation(latest, devices, deviceId, recoveryWords, recovery);
        const staged = await stagePreparedDeviceRevocation(
          latest, prepared, deviceId, idempotencyKey(), persistCredentials,
        );
        await commitPendingDeviceRevocation(
          staged,
          (request, key) => anywhereRequest(latest.serviceUrl ?? SERVICE_URL, `/v1/devices/${deviceId}/revoke`, {
            method: "POST",
            headers: { "Idempotency-Key": key },
            body: JSON.stringify(request),
          }, token),
          async () => deviceRevocationCommitted(latest.serviceUrl ?? SERVICE_URL, token, staged),
          persistCredentials,
        );
      });
      await mutationQueue.current;
      await refresh();
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [accessToken, devices, persistCredentials, refresh]);

  const enablePush = useCallback(async () => {
    try {
      setError(null);
      setPushStatus(await enableAnywherePush(pushApi));
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [pushApi]);

  const disablePush = useCallback(async () => {
    try {
      setError(null);
      setPushStatus(await disableAnywherePush(pushApi));
    } catch (reason) {
      setError(message(reason));
      throw reason;
    }
  }, [pushApi]);

  const logout = useCallback(async () => {
    const current = credentialsRef.current;
    if (current) {
      try {
        await anywhereRequest(current.serviceUrl ?? SERVICE_URL, "/v1/auth/logout", {
          method: "POST", body: JSON.stringify({ refresh_token: current.refreshToken }),
        }, current.accessToken);
      } catch { /* Local logout must still complete while offline. */ }
    }
    await credentialStore.clear();
    await clearAnywherePushState().catch(() => {
      // The service-side logout already revoked this device's subscriptions.
    });
    credentialsRef.current = null;
    setPushStatus("unsubscribed");
    setCredentials(null); setAccount(null); setSubscription(null); setHosts([]); setDevices([]); setFlow(null); setPending(null); setRecoverySetup(null); setError(null);
    if (current) await clearAnywhereHostCache(current.accountIdHex).catch(() => {
      // Protected credentials are already cleared; stale metadata can be overwritten next login.
    });
    await syncAnywhereHosts([]);
    setPhase("signed_out");
  }, [syncAnywhereHosts]);

  const value: AnywhereContextValue = {
    phase, credentials, account, subscription, hosts, devices, flow,
    recoveryWords: recoverySetup?.words ?? null,
    recoverySample: [3, 11, 20], error, pushStatus,
    accessToken, startLogin, confirmNewRecovery, recoverExisting, refresh, checkout, openBillingPortal,
    revokeDevice, revokeHost, selectHost, enablePush, disablePush, logout,
  };
  return <AnywhereContext.Provider value={value}>{children}</AnywhereContext.Provider>;
}

export function useAnywhere(): AnywhereContextValue {
  const value = useContext(AnywhereContext);
  if (!value) throw new Error("useAnywhere must be used within AnywhereProvider");
  return value;
}

function message(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Forge Anywhere could not complete the request";
}

async function deviceRevocationCommitted(
  serviceUrl: string,
  token: string,
  pending: { epoch: number; request: { recovery_wrap_envelope: string } },
): Promise<boolean> {
  const response = await anywhereRequest<AnywhereRecoveryWrap>(
    serviceUrl, `/v1/key-epochs/${pending.epoch}/wraps/recovery`, {}, token,
  );
  return response.epoch === pending.epoch
    && response.recovery_wrap_envelope === pending.request.recovery_wrap_envelope;
}
