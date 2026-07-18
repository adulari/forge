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
  refreshPendingAnywhereAuth,
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
import {
  openBrowserAuthUrl,
  reserveBrowserAuthWindow,
  type ReservedBrowserAuthWindow,
} from "./anywhereExternalAuth";
import {
  AnywhereJobClient,
  type CreateSessionJob,
  type PendingRemoteJob,
} from "./anywhereJobs";
import { anywhereJobStore } from "./anywhereJobStore";
import {
  promoteCurrentDeviceWrap,
  type AnywhereCurrentDeviceWrap,
} from "./anywhereEpochRefresh";
import {
  pairingDetails,
  parsePairingChallenge,
  preparePairingApproval,
  submitPairingApproval,
} from "./anywherePairing";
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
  remoteJobs: PendingRemoteJob[];
  /** Returns a fresh short-lived token for first-party Anywhere clients; never persist it. */
  accessToken(): Promise<string>;
  startLogin(): Promise<void>;
  openLoginPage(): Promise<void>;
  confirmNewRecovery(answers: Record<number, string>): Promise<void>;
  recoverExisting(words: string): Promise<void>;
  refresh(): Promise<void>;
  checkout(period?: AnywhereBillingPeriod): Promise<void>;
  openBillingPortal(): Promise<void>;
  revokeDevice(deviceId: string, recoveryWords: string): Promise<void>;
  revokeHost(hostId: string): Promise<void>;
  approvePairing(challenge: string): Promise<void>;
  selectHost(hostId: string): void;
  queueRemoteJob(input: Omit<CreateSessionJob, "hostDeviceId">): Promise<PendingRemoteJob>;
  refreshRemoteJobs(): Promise<void>;
  enablePush(): Promise<void>;
  disablePush(): Promise<void>;
  logout(): Promise<void>;
}

const AnywhereContext = createContext<AnywhereContextValue | null>(null);
const credentialStore = anywhereCredentialStore();
const jobStore = anywhereJobStore();

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
  const [remoteJobs, setRemoteJobs] = useState<PendingRemoteJob[]>([]);
  const mutationQueue = useRef(Promise.resolve());
  const revocationRecoveryAttempt = useRef<string | null>(null);
  const browserAuthWindow = useRef<ReservedBrowserAuthWindow | null>(null);
  const enrollmentInFlight = useRef(false);

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
      dataKeyForEpoch: async (epoch) => {
        const latest = credentialsRef.current;
        if (!latest) throw new Error("Forge Anywhere is not signed in");
        const encoded = epoch === latest.keyEpoch
          ? latest.accountDataKeyHex
          : latest.dataKeyEpochs?.[String(epoch)];
        if (!encoded) throw new Error("Anywhere response uses an unavailable key epoch");
        return bytesFromHex(encoded);
      },
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

  const jobClient = useMemo(
    // Runtime callbacks dereference protected credentials only when a queued operation runs.
    // eslint-disable-next-line react-hooks/refs
    () => runtime ? new AnywhereJobClient(runtime, jobStore) : null,
    [runtime],
  );

  const refreshRemoteJobs = useCallback(async () => {
    if (!jobClient) { setRemoteJobs([]); return; }
    setRemoteJobs(await jobClient.resume());
  }, [jobClient]);

  useEffect(() => {
    if (!jobClient || phase !== "ready") return;
    let cancelled = false;
    void jobClient.resume().then((jobs) => { if (!cancelled) setRemoteJobs(jobs); }).catch(() => {
      // Exact ciphertext remains protected in the local queue for the next reconnect.
    });
    return () => { cancelled = true; };
  }, [jobClient, phase]);

  const queueRemoteJob = useCallback(async (
    input: Omit<CreateSessionJob, "hostDeviceId">,
  ): Promise<PendingRemoteJob> => {
    if (!jobClient) throw new Error("Forge Anywhere is not signed in");
    const host = hosts.find((candidate) => candidate.id === input.hostId);
    if (!host) throw new Error("Select an enrolled destination host");
    try {
      setError(null);
      const job = await jobClient.queueCreateSession({ ...input, hostDeviceId: host.device_id });
      setRemoteJobs(await jobStore.load());
      return job;
    } catch (reason) {
      // An offline send is still durably queued; surface that state without deleting ciphertext.
      setRemoteJobs(await jobStore.load().catch(() => []));
      setError(message(reason));
      throw reason;
    }
  }, [hosts, jobClient]);

  const refresh = useCallback(async () => {
    if (!credentialsRef.current) return;
    try {
      const token = await accessToken();
      const serviceUrl = credentialsRef.current?.serviceUrl ?? SERVICE_URL;
      const [nextAccount, nextSubscription, hostList, deviceList, currentWrap] = await Promise.all([
        anywhereRequest<AnywhereAccountStatus>(serviceUrl, "/v1/me", {}, token),
        anywhereRequest<AnywhereSubscription>(serviceUrl, "/v1/billing/subscription", {}, token),
        anywhereRequest<{ hosts: AnywhereHost[] }>(serviceUrl, "/v1/hosts", {}, token),
        anywhereRequest<{ devices: AnywhereDevice[] }>(serviceUrl, "/v1/devices", {}, token),
        anywhereRequest<AnywhereCurrentDeviceWrap>(serviceUrl, "/v1/key-epochs/current/wraps/device", {}, token),
      ]);
      const publicKeys = Object.fromEntries(deviceList.devices.map((device) => [device.id, bytesToHex(fromBase64Url(device.signing_public_key))]));
      mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
        const before = credentialsRef.current;
        if (!before) return;
        // Validate a rotation against the previously trusted signer set. Only after the key is
        // accepted do we replace the cached enrollment keys returned by the same refresh.
        const promoted = promoteCurrentDeviceWrap(before, currentWrap);
        const next = { ...promoted, signingPublicKeys: publicKeys };
        if (JSON.stringify(before) !== JSON.stringify(next)) await persistCredentials(next);
      });
      await mutationQueue.current;
      const latest = credentialsRef.current;
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
    browserAuthWindow.current?.close();
    const reservedWindow = Platform.OS === "web" ? reserveBrowserAuthWindow() : null;
    browserAuthWindow.current = reservedWindow;
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
      if (Platform.OS === "web") {
        reservedWindow?.navigate(started.verification_uri);
      } else {
        void Linking.openURL(started.verification_uri).catch(() => {
          // The code and a retryable link remain visible if the OS cannot open the browser.
        });
      }
    } catch (reason) {
      reservedWindow?.close();
      if (browserAuthWindow.current === reservedWindow) browserAuthWindow.current = null;
      setError(message(reason));
      setPhase("error");
    }
  }, []);

  const openLoginPage = useCallback(async () => {
    if (!flow) return;
    if (Platform.OS === "web") {
      openBrowserAuthUrl(flow.verification_uri);
      return;
    }
    await Linking.openURL(flow.verification_uri);
  }, [flow]);

  useEffect(() => {
    if (phase !== "authorizing" || !flow || !pending) return;
    let cancelled = false;
    const deadline = Date.now() + flow.expires_in * 1000;
    const timer = setInterval(() => {
      if (cancelled) return;
      if (Date.now() >= deadline) {
        clearInterval(timer);
        browserAuthWindow.current?.close();
        browserAuthWindow.current = null;
        setError("GitHub login expired. Start again to receive a new code.");
        setPhase("error");
        return;
      }
      void anywhereRequest<AnywhereAuthSession | undefined>(SERVICE_URL, "/v1/auth/device/poll", {
        method: "POST", body: JSON.stringify({ device_code: flow.device_code }),
      }).then((session) => {
        if (cancelled || !session) return;
        clearInterval(timer);
        browserAuthWindow.current?.close();
        browserAuthWindow.current = null;
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
        if (!cancelled) {
          clearInterval(timer);
          browserAuthWindow.current?.close();
          browserAuthWindow.current = null;
          setError(message(reason));
          setPhase("error");
        }
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
    if (!recoverySetup || enrollmentInFlight.current) return;
    const words = recoverySetup.words.split(" ");
    const samples = [3, 11, 20];
    if (samples.some((index) => answers[index]?.trim().toLowerCase() !== words[index])) {
      setError("Those recovery words do not match. Check the numbered words and try again.");
      return;
    }
    enrollmentInFlight.current = true;
    try {
      setError(null);
      const auth = await refreshPendingAnywhereAuth(
        recoverySetup.auth,
        (refreshToken) => anywhereRequest(SERVICE_URL, "/v1/auth/refresh", {
          method: "POST",
          body: JSON.stringify({ refresh_token: refreshToken }),
        }),
      );
      const latestSetup = auth === recoverySetup.auth ? recoverySetup : { ...recoverySetup, auth };
      if (latestSetup !== recoverySetup) {
        setRecoverySetup(latestSetup);
        setPending(latestSetup);
      }
      const accountId = bytesFromHex(auth.account_id);
      const deviceId = bytesFromHex(auth.device_id);
      const deviceKey = deriveSelfDeviceWrapKey(latestSetup.keys.exchangePrivateKey, latestSetup.keys.exchangePublicKey, accountId, INITIAL_EPOCH);
      const recoveryKey = deriveRecoveryWrapKey(latestSetup.entropy, accountId, INITIAL_EPOCH);
      const deviceWrap = makeKeyWrap(latestSetup.dataKey, deviceKey, accountId, deviceId, 1, deviceId, INITIAL_EPOCH, 0n, latestSetup.keys.signingPrivateKey);
      const recoveryWrap = makeKeyWrap(latestSetup.dataKey, recoveryKey, accountId, deviceId, 3, accountId, INITIAL_EPOCH, 1n, latestSetup.keys.signingPrivateKey);
      await anywhereRequest(SERVICE_URL, "/v1/key-epochs", {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ epoch: INITIAL_EPOCH, device_wrap_envelope: base64Url(deviceWrap), recovery_wrap_envelope: base64Url(recoveryWrap) }),
      }, auth.access_token);
      await finishEnrollment(latestSetup, latestSetup.dataKey, INITIAL_EPOCH, 2n);
    } catch (reason) {
      setError(message(reason));
    } finally {
      enrollmentInFlight.current = false;
    }
  }, [finishEnrollment, recoverySetup]);

  const recoverExisting = useCallback(async (words: string) => {
    if (enrollmentInFlight.current) return;
    if (!pending?.auth.recovery_wrap_envelope || !pending.auth.recovery_wrap_signing_public_key) {
      setError("The service did not return this account's encrypted recovery key.");
      return;
    }
    enrollmentInFlight.current = true;
    try {
      setError(null);
      const auth = await refreshPendingAnywhereAuth(
        pending.auth,
        (refreshToken) => anywhereRequest(SERVICE_URL, "/v1/auth/refresh", {
          method: "POST",
          body: JSON.stringify({ refresh_token: refreshToken }),
        }),
      );
      const latestPending = auth === pending.auth ? pending : { ...pending, auth };
      if (latestPending !== pending) setPending(latestPending);
      const recovered = openRecoveryWrap(auth.recovery_wrap_envelope!, auth.recovery_wrap_signing_public_key!, words, auth.account_id);
      const accountId = bytesFromHex(auth.account_id);
      const deviceId = bytesFromHex(auth.device_id);
      const deviceKey = deriveSelfDeviceWrapKey(pending.keys.exchangePrivateKey, pending.keys.exchangePublicKey, accountId, recovered.epoch);
      const wrap = makeKeyWrap(recovered.dataKey, deviceKey, accountId, deviceId, 1, deviceId, recovered.epoch, 0n, pending.keys.signingPrivateKey);
      await anywhereRequest(SERVICE_URL, `/v1/key-epochs/${recovered.epoch}/wraps`, {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ epoch: recovered.epoch, device_wrap_envelope: base64Url(wrap) }),
      }, auth.access_token);
      await finishEnrollment(latestPending, recovered.dataKey, recovered.epoch, 1n);
    } catch (reason) {
      setError(message(reason));
    } finally {
      enrollmentInFlight.current = false;
    }
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

  const approvePairing = useCallback(async (encodedChallenge: string) => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in");
    const serviceUrl = current.serviceUrl ?? SERVICE_URL;
    const challenge = parsePairingChallenge(encodedChallenge, serviceUrl);
    const token = await accessToken();
    const details = await pairingDetails(serviceUrl, token, challenge);
    let approval: ReturnType<typeof preparePairingApproval> | null = null;
    mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
      const latest = credentialsRef.current;
      if (!latest) throw new Error("Forge Anywhere is not signed in");
      const sequence = BigInt(latest.nextSequence);
      approval = preparePairingApproval(latest, challenge, details, sequence);
      // Reserve before approval so a crash can never reuse this signed epoch/sequence tuple.
      await persistCredentials({ ...latest, nextSequence: (sequence + 1n).toString() });
    });
    await mutationQueue.current;
    if (!approval) throw new Error("Forge Anywhere pairing approval could not be prepared");
    await submitPairingApproval(serviceUrl, token, challenge.pairing_id, approval);
    await refresh();
  }, [accessToken, persistCredentials, refresh]);

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
    setCredentials(null); setAccount(null); setSubscription(null); setHosts([]); setDevices([]); setRemoteJobs([]); setFlow(null); setPending(null); setRecoverySetup(null); setError(null);
    if (current) await clearAnywhereHostCache(current.accountIdHex).catch(() => {
      // Protected credentials are already cleared; stale metadata can be overwritten next login.
    });
    await syncAnywhereHosts([]);
    setPhase("signed_out");
  }, [syncAnywhereHosts]);

  const value: AnywhereContextValue = {
    phase, credentials, account, subscription, hosts, devices, flow,
    recoveryWords: recoverySetup?.words ?? null,
    recoverySample: [3, 11, 20], error, pushStatus, remoteJobs,
    accessToken, startLogin, openLoginPage, confirmNewRecovery, recoverExisting, refresh, checkout, openBillingPortal,
    revokeDevice, revokeHost, selectHost, approvePairing, queueRemoteJob, refreshRemoteJobs,
    enablePush, disablePush, logout,
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
