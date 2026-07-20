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
  type AnywhereResetStatus,
  type AnywhereSubscription,
  anywhereRequest,
  base64Url,
  fromBase64Url,
  idempotencyKey,
  preflightAnywhere,
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
  createRecoveryKitV2,
  deriveRecoveryWrapKey,
  deriveSelfDeviceWrapKey,
  generatePendingKeys,
  generateRecoveryPhrase,
  makeKeyWrap,
  openRecoveryWrap,
  openRecoveryWrapWithEntropy,
  recoveryEntropyFromInput,
  type PendingAnywhereKeys,
} from "./anywhereCrypto";
import { clearAnywhereHostCache, readAnywhereHostCache, writeAnywhereHostCache } from "./anywhereHostCache";
import { anywhereEnrollmentStore } from "./anywhereEnrollmentStore";
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
import { secureRandomBytes } from "./secureRandom";
import {
  promoteCurrentDeviceWrap,
  type AnywhereCurrentDeviceWrap,
} from "./anywhereEpochRefresh";
import {
  challengeFromDetails,
  cancelPairing,
  createEnrollmentRequest,
  createPairing,
  denyPairing,
  listPairings,
  openApprovedPairing,
  pairingDetails,
  pairingSafetyCode,
  parsePairingChallenge,
  pollPairing,
  preparePairingApproval,
  submitPairingApproval,
  type PairingChallenge,
  type PairingCreateResponse,
  type PairingDetails,
} from "./anywherePairing";
import {
  createPasskeySession,
  generatePasskeyExchange,
  getPasskeySession,
  listPasskeys,
  openPasskeySecret,
  passkeyChannelAad,
  passkeyChannelKey,
  renamePasskey,
  revokePasskey,
  sealPasskeySecret,
  sendRegistrationEntropy,
  type AnywherePasskey,
} from "./anywherePasskeys";
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
import { anywhereConsumersReady } from "./anywhereStartup";

const SERVICE_URL = process.env.EXPO_PUBLIC_FORGE_ANYWHERE_URL ?? DEFAULT_ANYWHERE_SERVICE_URL;
const INITIAL_EPOCH = 1;

type PendingLogin = { auth: AnywhereAuthSession; keys: PendingAnywhereKeys };
type RecoverySetup = PendingLogin & { words: string; entropy: Uint8Array; dataKey: Uint8Array };
type PendingClaimantPairing = {
  login: PendingLogin;
  created: PairingCreateResponse;
  challenge: PairingChallenge;
  safetyCode: string;
};

interface EnrollmentSnapshot {
  version: 1;
  kind: "authorizing" | "awaiting_approval" | "new_recovery" | "existing_recovery";
  auth?: AnywhereAuthSession;
  keys: { signingPrivate: string; exchangePrivate: string; signingPublic: string; exchangePublic: string };
  flow?: AnywhereDeviceFlow;
  flowExpiresAtMs?: number;
  created?: PairingCreateResponse;
  challenge?: PairingChallenge;
  safetyCode?: string;
  words?: string;
  entropy?: string;
  dataKey?: string;
}

export interface AnywherePendingApproval {
  id: string;
  deviceId: string;
  deviceName: string;
  expiresAtMs: number;
  safetyCode: string;
}

export interface AnywhereClaimantApproval {
  expiresAtMs: number;
  safetyCode: string;
  deviceName: string;
  githubLogin: string;
}

export interface AnywhereLocalHostApproval {
  name: string;
  expiresAtMs: number;
  safetyCode: string;
}

type PendingLocalHost = {
  name: string;
  keys: PendingAnywhereKeys;
  created: PairingCreateResponse;
  challenge: PairingChallenge;
  details: PairingDetails;
  safetyCode: string;
};

export type AnywherePhase =
  | "loading"
  | "signed_out"
  | "starting"
  | "authorizing"
  | "awaiting_approval"
  | "new_recovery"
  | "existing_recovery"
  | "ready"
  | "error";

export interface AnywhereContextValue {
  phase: AnywherePhase;
  credentials: StoredAnywhereCredentials | null;
  account: AnywhereAccountStatus | null;
  subscription: AnywhereSubscription | null;
  hosts: AnywhereHost[];
  devices: AnywhereDevice[];
  flow: AnywhereDeviceFlow | null;
  recoveryWords: string | null;
  recoveryKit: string | null;
  recoverySample: readonly number[];
  claimantApproval: AnywhereClaimantApproval | null;
  localHostApproval: AnywhereLocalHostApproval | null;
  pendingApprovals: AnywherePendingApproval[];
  passkeys: AnywherePasskey[];
  approvalError: string | null;
  error: string | null;
  pushStatus: AnywherePushStatus;
  remoteJobs: PendingRemoteJob[];
  /** Returns a fresh short-lived token for first-party Anywhere clients; never persist it. */
  accessToken(): Promise<string>;
  startLogin(): Promise<void>;
  openLoginPage(): Promise<void>;
  confirmNewRecovery(answers: Record<number, string>): Promise<void>;
  recoverExisting(words: string): Promise<void>;
  scheduleCleanReset(confirmation: string): Promise<number>;
  cancelCleanReset(): Promise<void>;
  registerPasskey(recoveryKit: string): Promise<void>;
  recoverWithPasskey(passkeyId: string): Promise<void>;
  renamePasskey(passkeyId: string, name: string): Promise<void>;
  revokePasskey(passkeyId: string): Promise<void>;
  useRecoveryInstead(): void;
  restartSetup(): void;
  refresh(): Promise<void>;
  checkout(period?: AnywhereBillingPeriod): Promise<void>;
  openBillingPortal(): Promise<void>;
  revokeDevice(deviceId: string, recoveryWords: string): Promise<void>;
  revokeHost(hostId: string): Promise<void>;
  renameHost(hostId: string, name: string): Promise<void>;
  approvePairing(challenge: string): Promise<void>;
  approvePendingDevice(pairingId: string): Promise<void>;
  denyPendingDevice(pairingId: string): Promise<void>;
  refreshPendingApprovals(): Promise<void>;
  prepareLocalHost(name: string): Promise<"approval" | "activated">;
  confirmLocalHost(): Promise<void>;
  cancelLocalHost(): void;
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
const enrollmentStore = anywhereEnrollmentStore();

export function AnywhereProvider({ children }: { children: React.ReactNode }) {
  const auth = useAuth();
  const { setActive, syncAnywhereHosts } = auth;
  const [phase, setPhase] = useState<AnywherePhase>("loading");
  const [credentials, setCredentials] = useState<StoredAnywhereCredentials | null>(null);
  const [registeredRuntimeId, setRegisteredRuntimeId] = useState<string | null>(null);
  const credentialsRef = useRef<StoredAnywhereCredentials | null>(null);
  const [account, setAccount] = useState<AnywhereAccountStatus | null>(null);
  const [subscription, setSubscription] = useState<AnywhereSubscription | null>(null);
  const [hosts, setHosts] = useState<AnywhereHost[]>([]);
  const [devices, setDevices] = useState<AnywhereDevice[]>([]);
  const [flow, setFlow] = useState<AnywhereDeviceFlow | null>(null);
  const [flowExpiresAtMs, setFlowExpiresAtMs] = useState<number | null>(null);
  const [pending, setPending] = useState<PendingLogin | null>(null);
  const [recoverySetup, setRecoverySetup] = useState<RecoverySetup | null>(null);
  const [claimantPairing, setClaimantPairing] = useState<PendingClaimantPairing | null>(null);
  const [pendingApprovalDetails, setPendingApprovalDetails] = useState<PairingDetails[]>([]);
  const [passkeys, setPasskeys] = useState<AnywherePasskey[]>([]);
  const [pendingLocalHost, setPendingLocalHost] = useState<PendingLocalHost | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
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
      randomBytes: secureRandomBytes,
    };
    // Runtime remains stable while tokens/sequences mutate; a key/device epoch change replaces it.
    // Only key identity changes replace live relays; token and replay-state writes are read via refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accessToken, credentials?.accountDataKeyHex, credentials?.accountIdHex, credentials?.deviceIdHex, credentials?.keyEpoch, credentials?.serviceUrl, credentials?.signingPrivateKeyHex]);
  /* eslint-enable react-hooks/refs */
  const runtimeId = credentials
    ? [
        credentials.serviceUrl ?? SERVICE_URL,
        credentials.accountIdHex,
        credentials.deviceIdHex,
        credentials.keyEpoch,
        credentials.accountDataKeyHex,
        credentials.signingPrivateKeyHex,
      ].join(":")
    : null;

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
      const [nextAccount, nextSubscription, hostList, deviceList, currentWrap, nextPasskeys] = await Promise.all([
        anywhereRequest<AnywhereAccountStatus>(serviceUrl, "/v1/me", {}, token),
        anywhereRequest<AnywhereSubscription>(serviceUrl, "/v1/billing/subscription", {}, token),
        anywhereRequest<{ hosts: AnywhereHost[] }>(serviceUrl, "/v1/hosts", {}, token),
        anywhereRequest<{ devices: AnywhereDevice[] }>(serviceUrl, "/v1/devices", {}, token),
        anywhereRequest<AnywhereCurrentDeviceWrap>(serviceUrl, "/v1/key-epochs/current/wraps/device", {}, token),
        listPasskeys(serviceUrl, token),
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
      setPasskeys(nextPasskeys);
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
      if (loaded) {
        setHosts(cachedHosts);
        setPhase("ready");
        void refresh();
        return;
      }
      const serialized = await enrollmentStore.load();
      if (cancelled) return;
      if (!serialized) {
        setPhase("signed_out");
        return;
      }
      const snapshot = parseEnrollmentSnapshot(serialized);
      const keys = restorePendingKeys(snapshot.keys);
      if (snapshot.kind === "authorizing" && snapshot.flow && snapshot.flowExpiresAtMs && snapshot.flowExpiresAtMs > Date.now()) {
        setPending({ auth: null as never, keys });
        setFlow(snapshot.flow);
        setFlowExpiresAtMs(snapshot.flowExpiresAtMs);
        setPhase("authorizing");
        return;
      }
      if (!snapshot.auth) throw new Error("protected Forge Anywhere enrollment session is incomplete");
      const login = { auth: snapshot.auth, keys };
      setPending(login);
      if (snapshot.kind === "awaiting_approval" && snapshot.created && snapshot.challenge && snapshot.safetyCode) {
        setClaimantPairing({ login, created: snapshot.created, challenge: snapshot.challenge, safetyCode: snapshot.safetyCode });
        setPhase("awaiting_approval");
      } else if (snapshot.kind === "new_recovery" && snapshot.words && snapshot.entropy && snapshot.dataKey) {
        setRecoverySetup({ ...login, words: snapshot.words, entropy: bytesFromHex(snapshot.entropy), dataKey: bytesFromHex(snapshot.dataKey) });
        setPhase("new_recovery");
      } else if (snapshot.kind === "existing_recovery") {
        setPhase("existing_recovery");
      } else {
        throw new Error("protected Forge Anywhere enrollment state is incomplete");
      }
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
    if (!runtime || phase !== "ready" || auth.isLoading) {
      if (!runtime) setRegisteredRuntimeId(null);
      return;
    }
    let active = true;
    const removers = hosts.map((host) => registerAnywhereTransport(
      new AnywhereTransport(host.id, new EncryptedAnywhereRelay(runtime)),
    ));
    void syncAnywhereHosts(hosts.map((host) => ({ id: host.id, name: host.name }))).then(() => {
      // Do not mount consumers while Auth still points at an unreconciled managed target.
      if (active) setRegisteredRuntimeId(runtimeId);
    });
    return () => {
      active = false;
      for (const remove of removers) remove();
    };
  }, [auth.isLoading, hosts, phase, runtime, runtimeId, syncAnywhereHosts]);

  const startLogin = useCallback(async () => {
    browserAuthWindow.current?.close();
    const reservedWindow = Platform.OS === "web" ? reserveBrowserAuthWindow() : null;
    browserAuthWindow.current = reservedWindow;
    setError(null);
    setPhase("starting");
    try {
      await preflightAnywhere(SERVICE_URL);
      const keys = generatePendingKeys();
      const started = await anywhereRequest<AnywhereDeviceFlow>(SERVICE_URL, "/v1/auth/github/start", {
        method: "POST",
        body: JSON.stringify({
          device_name: deviceName(),
          signing_public_key: base64Url(keys.signingPublicKey),
          exchange_public_key: base64Url(keys.exchangePublicKey),
        }),
      });
      setPending({ auth: null as never, keys });
      setFlow(started);
      const expiresAtMs = Date.now() + started.expires_in * 1000;
      setFlowExpiresAtMs(expiresAtMs);
      await enrollmentStore.save(JSON.stringify(snapshotAuthorizing(keys, started, expiresAtMs)));
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
    const deadline = flowExpiresAtMs ?? Date.now() + flow.expires_in * 1000;
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
      }).then(async (session) => {
        if (cancelled || !session) return;
        clearInterval(timer);
        browserAuthWindow.current?.close();
        browserAuthWindow.current = null;
        const nextPending = { auth: session, keys: pending.keys };
        setPending(nextPending);
        if (!session.new_account) {
          void listPasskeys(SERVICE_URL, session.access_token).then(setPasskeys).catch(() => setPasskeys([]));
        }
        if (session.new_account) {
          const recovery = generateRecoveryPhrase();
          const setup = { ...nextPending, ...recovery, dataKey: secureRandomBytes(32) };
          await enrollmentStore.save(JSON.stringify(snapshotRecovery(setup)));
          setRecoverySetup(setup);
          setPhase("new_recovery");
        } else {
          const request = {
            version: 1 as const,
            device_name: deviceName(),
            signing_public_key: base64Url(pending.keys.signingPublicKey),
            exchange_public_key: base64Url(pending.keys.exchangePublicKey),
          };
          const created = await createEnrollmentRequest(
            SERVICE_URL,
            session.access_token,
            request,
          );
          const challenge = parsePairingChallenge(created.challenge, SERVICE_URL);
          const pairing = {
            login: nextPending,
            created,
            challenge,
            safetyCode: pairingSafetyCode(challenge, request.signing_public_key, session.account_id),
          };
          await enrollmentStore.save(JSON.stringify(snapshotPairing(pairing)));
          setClaimantPairing(pairing);
          setPhase("awaiting_approval");
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
  }, [flow, flowExpiresAtMs, pending, phase]);

  const finishEnrollment = useCallback(async (
    login: PendingLogin,
    dataKey: Uint8Array,
    epoch: number,
    nextSequence: bigint,
    recoveryKitVerified = false,
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
      recoveryKitVerified,
    };
    await persistCredentials(next);
    await enrollmentStore.clear();
    setPending(null);
    setRecoverySetup(null);
    setClaimantPairing(null);
    setFlow(null);
    setFlowExpiresAtMs(null);
    setPhase("ready");
    await refresh();
  }, [persistCredentials, refresh]);

  useEffect(() => {
    if (phase !== "awaiting_approval" || !claimantPairing) return;
    let cancelled = false;
    let checking = false;
    const check = async () => {
      if (cancelled || checking) return;
      if (Date.now() >= claimantPairing.created.expires_at_ms) {
        setError("Device approval request expired. Start again or use your Recovery Kit.");
        setPhase("error");
        return;
      }
      checking = true;
      try {
        const result = await pollPairing(
          SERVICE_URL,
          claimantPairing.created.pairing_id,
          claimantPairing.created.pairing_token,
        );
        if (cancelled || result.status === "pending") return;
        if (result.status === "denied") {
          setError("Approval denied. Check the device name before starting again.");
          setPhase("error");
          return;
        }
        if (result.account_id !== claimantPairing.login.auth.account_id) {
          throw new Error("Approval came from a different Forge Anywhere account. Nothing was installed.");
        }
        const opened = openApprovedPairing(result, claimantPairing.login.keys.exchangePrivateKey);
        const approvedLogin: PendingLogin = {
          keys: claimantPairing.login.keys,
          auth: {
            ...claimantPairing.login.auth,
            account_id: result.account_id,
            device_id: result.device_id,
            access_token: result.access_token,
            refresh_token: result.refresh_token,
            access_expires_at_ms: result.access_expires_at_ms,
          },
        };
        await finishEnrollment(approvedLogin, opened.accountDataKey, opened.epoch, 0n);
      } catch (reason) {
        if (!cancelled) {
          setError(message(reason));
          setPhase("error");
        }
      } finally {
        checking = false;
      }
    };
    void check();
    const timer = setInterval(() => void check(), 2_000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [claimantPairing, finishEnrollment, phase]);

  const confirmNewRecovery = useCallback(async (answers: Record<number, string>) => {
    if (!recoverySetup || enrollmentInFlight.current) return;
    const words = recoverySetup.words.split(" ");
    const samples = [2, 6, 8];
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
      await finishEnrollment(latestSetup, latestSetup.dataKey, INITIAL_EPOCH, 2n, true);
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
      const recovered = openRecoveryWrap(auth.recovery_wrap_envelope!, auth.recovery_wrap_signing_public_key!, words, auth.account_id, SERVICE_URL);
      const accountId = bytesFromHex(auth.account_id);
      const deviceId = bytesFromHex(auth.device_id);
      const deviceKey = deriveSelfDeviceWrapKey(pending.keys.exchangePrivateKey, pending.keys.exchangePublicKey, accountId, recovered.epoch);
      const wrap = makeKeyWrap(recovered.dataKey, deviceKey, accountId, deviceId, 1, deviceId, recovered.epoch, 0n, pending.keys.signingPrivateKey);
      await anywhereRequest(SERVICE_URL, `/v1/key-epochs/${recovered.epoch}/wraps`, {
        method: "POST",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ epoch: recovered.epoch, device_wrap_envelope: base64Url(wrap) }),
      }, auth.access_token);
      await finishEnrollment(latestPending, recovered.dataKey, recovered.epoch, 1n, true);
    } catch (reason) {
      setError(message(reason));
    } finally {
      enrollmentInFlight.current = false;
    }
  }, [finishEnrollment, pending]);

  const scheduleCleanReset = useCallback(async (confirmation: string) => {
    if (!pending?.auth) throw new Error("Sign in with GitHub again before scheduling a reset.");
    const auth = await refreshPendingAnywhereAuth(
      pending.auth,
      (refreshToken) => anywhereRequest(SERVICE_URL, "/v1/auth/refresh", {
        method: "POST",
        body: JSON.stringify({ refresh_token: refreshToken }),
      }),
    );
    if (auth !== pending.auth) setPending({ ...pending, auth });
    const status = await anywhereRequest<AnywhereResetStatus>(SERVICE_URL, "/v1/account/reset", {
      method: "POST",
      body: JSON.stringify({ version: 1, confirmation }),
    }, auth.access_token);
    if (!status.pending_reset) throw new Error("Clean reset was not scheduled.");
    return status.pending_reset.executes_at_ms;
  }, [pending]);

  const cancelCleanReset = useCallback(async () => {
    const current = credentialsRef.current;
    if (!current) throw new Error("An enrolled device is required to cancel this reset.");
    const token = await accessToken();
    await anywhereRequest(current.serviceUrl ?? SERVICE_URL, "/v1/account/reset/cancel", {
      method: "POST",
    }, token);
    await refresh();
  }, [accessToken, refresh]);

  const registerRecoveryPasskey = useCallback(async (recoveryKit: string) => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Sign in to register a recovery passkey.");
    const reserved = Platform.OS === "web" ? reserveBrowserAuthWindow() : null;
    const entropy = recoveryEntropyFromInput(
      recoveryKit,
      current.serviceUrl ?? SERVICE_URL,
      current.accountIdHex,
    );
    if (Platform.OS === "web" && !reserved) {
      entropy.fill(0);
      throw new Error("Allow Forge to open a browser tab for passkey registration.");
    }
    try {
      const token = await accessToken();
      const exchange = generatePasskeyExchange();
      const created = await createPasskeySession(
        current.serviceUrl ?? SERVICE_URL,
        token,
        "registration",
        exchange.publicKey,
      );
      if (Platform.OS === "web") reserved?.navigate(created.browser_url);
      else await Linking.openURL(created.browser_url);
      let payloadSent = false;
      while (Date.now() < created.expires_at_ms) {
        const session = await getPasskeySession(current.serviceUrl ?? SERVICE_URL, created.session_token);
        if (session.account_id !== current.accountIdHex) throw new Error("Passkey session belongs to another account.");
        if (session.browser_exchange_public_key && !payloadSent) {
          const key = passkeyChannelKey(
            exchange.privateKey,
            fromBase64Url(session.browser_exchange_public_key),
            current.accountIdHex,
            created.session_token,
          );
          await sendRegistrationEntropy(
            current.serviceUrl ?? SERVICE_URL,
            token,
            created.session_token,
            sealPasskeySecret(
              entropy,
              key,
              passkeyChannelAad(current.accountIdHex, "registration"),
            ),
          );
          key.fill(0);
          payloadSent = true;
        }
        if (session.completed) {
          setPasskeys(await listPasskeys(current.serviceUrl ?? SERVICE_URL, token));
          return;
        }
        await delay(1_500);
      }
      throw new Error("Passkey registration expired. Try again.");
    } finally {
      entropy.fill(0);
    }
  }, [accessToken]);

  const recoverWithPasskey = useCallback(async (passkeyId: string) => {
    if (!pending?.auth.recovery_wrap_envelope || !pending.auth.recovery_wrap_signing_public_key) {
      throw new Error("The encrypted recovery key is unavailable for this account.");
    }
    const reserved = Platform.OS === "web" ? reserveBrowserAuthWindow() : null;
    const auth = await refreshPendingAnywhereAuth(
      pending.auth,
      (refreshToken) => anywhereRequest(SERVICE_URL, "/v1/auth/refresh", {
        method: "POST",
        body: JSON.stringify({ refresh_token: refreshToken }),
      }),
    );
    const latestPending = auth === pending.auth ? pending : { ...pending, auth };
    if (latestPending !== pending) setPending(latestPending);
    const exchange = generatePasskeyExchange();
    const created = await createPasskeySession(
      SERVICE_URL,
      auth.access_token,
      "authentication",
      exchange.publicKey,
      passkeyId,
    );
    if (Platform.OS === "web") {
      if (!reserved) throw new Error("Allow Forge to open a browser tab for passkey recovery.");
      reserved.navigate(created.browser_url);
    } else await Linking.openURL(created.browser_url);
    while (Date.now() < created.expires_at_ms) {
      const session = await getPasskeySession(SERVICE_URL, created.session_token);
      if (session.account_id !== auth.account_id) throw new Error("Passkey session belongs to another account.");
      if (session.to_client_ciphertext && session.browser_exchange_public_key) {
        const key = passkeyChannelKey(
          exchange.privateKey,
          fromBase64Url(session.browser_exchange_public_key),
          auth.account_id,
          created.session_token,
        );
        const entropy = openPasskeySecret(
          session.to_client_ciphertext,
          key,
          passkeyChannelAad(auth.account_id, "authentication"),
        );
        const recovered = openRecoveryWrapWithEntropy(
          auth.recovery_wrap_envelope!,
          auth.recovery_wrap_signing_public_key!,
          entropy,
          auth.account_id,
        );
        entropy.fill(0);
        key.fill(0);
        const accountId = bytesFromHex(auth.account_id);
        const deviceId = bytesFromHex(auth.device_id);
        const deviceKey = deriveSelfDeviceWrapKey(
          latestPending.keys.exchangePrivateKey,
          latestPending.keys.exchangePublicKey,
          accountId,
          recovered.epoch,
        );
        const wrap = makeKeyWrap(
          recovered.dataKey,
          deviceKey,
          accountId,
          deviceId,
          1,
          deviceId,
          recovered.epoch,
          0n,
          latestPending.keys.signingPrivateKey,
        );
        await anywhereRequest(SERVICE_URL, `/v1/key-epochs/${recovered.epoch}/wraps`, {
          method: "POST",
          headers: { "Idempotency-Key": idempotencyKey() },
          body: JSON.stringify({ epoch: recovered.epoch, device_wrap_envelope: base64Url(wrap) }),
        }, auth.access_token);
        await finishEnrollment(latestPending, recovered.dataKey, recovered.epoch, 1n, true);
        return;
      }
      await delay(1_500);
    }
    throw new Error("Passkey recovery expired. Try again.");
  }, [finishEnrollment, pending]);

  const renameRecoveryPasskey = useCallback(async (passkeyId: string, name: string) => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in.");
    const token = await accessToken();
    await renamePasskey(current.serviceUrl ?? SERVICE_URL, token, passkeyId, name);
    setPasskeys(await listPasskeys(current.serviceUrl ?? SERVICE_URL, token));
  }, [accessToken]);

  const revokeRecoveryPasskey = useCallback(async (passkeyId: string) => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in.");
    const token = await accessToken();
    await revokePasskey(current.serviceUrl ?? SERVICE_URL, token, passkeyId);
    setPasskeys(await listPasskeys(current.serviceUrl ?? SERVICE_URL, token));
  }, [accessToken]);

  const useRecoveryInstead = useCallback(() => {
    if (!pending?.auth || pending.auth.new_account) return;
    setError(null);
    void enrollmentStore.save(JSON.stringify(snapshotExistingRecovery(pending)));
    setPhase("existing_recovery");
  }, [pending]);

  const restartSetup = useCallback(() => {
    if (claimantPairing) {
      void cancelPairing(
        SERVICE_URL,
        claimantPairing.created.pairing_id,
        claimantPairing.created.pairing_token,
      ).catch(() => {
        // Expiry and prior denial are already safe terminal states.
      });
    }
    browserAuthWindow.current?.close();
    browserAuthWindow.current = null;
    setFlow(null);
    setFlowExpiresAtMs(null);
    setPending(null);
    setRecoverySetup(null);
    setClaimantPairing(null);
    setError(null);
    void enrollmentStore.clear();
    setPhase(credentialsRef.current ? "ready" : "signed_out");
  }, [claimantPairing]);

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

  const renameHost = useCallback(async (hostId: string, name: string) => {
    const normalized = name.trim();
    if (!normalized) throw new Error("Enter a host name");
    if (normalized.length > 80) throw new Error("Host names can contain at most 80 characters");
    try {
      setError(null);
      const token = await accessToken();
      const current = credentialsRef.current;
      if (!current) throw new Error("Forge Anywhere is not signed in");
      await anywhereRequest(current.serviceUrl ?? SERVICE_URL, `/v1/hosts/${hostId}`, {
        method: "PATCH",
        headers: { "Idempotency-Key": idempotencyKey() },
        body: JSON.stringify({ name: normalized }),
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

  const refreshPendingApprovals = useCallback(async () => {
    if (!credentialsRef.current) {
      setPendingApprovalDetails([]);
      return;
    }
    try {
      const token = await accessToken();
      const serviceUrl = credentialsRef.current?.serviceUrl ?? SERVICE_URL;
      setPendingApprovalDetails(await listPairings(serviceUrl, token));
      setApprovalError(null);
    } catch (reason) {
      setApprovalError(message(reason));
    }
  }, [accessToken]);

  useEffect(() => {
    if (phase !== "ready" || !credentials?.deviceIdHex) {
      setPendingApprovalDetails([]);
      return;
    }
    void refreshPendingApprovals();
    const timer = setInterval(() => void refreshPendingApprovals(), 15_000);
    return () => clearInterval(timer);
  }, [credentials?.deviceIdHex, phase, refreshPendingApprovals]);

  const approvePendingDevice = useCallback(async (pairingId: string) => {
    const details = pendingApprovalDetails.find((candidate) => candidate.pairing_id === pairingId);
    const current = credentialsRef.current;
    if (!details || !current) throw new Error("Device approval request is no longer pending");
    const serviceUrl = current.serviceUrl ?? SERVICE_URL;
    const challenge = challengeFromDetails(details, serviceUrl);
    const token = await accessToken();
    let approval: ReturnType<typeof preparePairingApproval> | null = null;
    mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
      const latest = credentialsRef.current;
      if (!latest) throw new Error("Forge Anywhere is not signed in");
      const sequence = BigInt(latest.nextSequence);
      approval = preparePairingApproval(latest, challenge, details, sequence);
      await persistCredentials({ ...latest, nextSequence: (sequence + 1n).toString() });
    });
    await mutationQueue.current;
    if (!approval) throw new Error("Device approval could not be prepared");
    await submitPairingApproval(serviceUrl, token, pairingId, approval);
    setPendingApprovalDetails((currentDetails) => currentDetails.filter((candidate) => candidate.pairing_id !== pairingId));
    await refresh();
  }, [accessToken, pendingApprovalDetails, persistCredentials, refresh]);

  const denyPendingDevice = useCallback(async (pairingId: string) => {
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in");
    const token = await accessToken();
    await denyPairing(current.serviceUrl ?? SERVICE_URL, token, pairingId);
    setPendingApprovalDetails((currentDetails) => currentDetails.filter((candidate) => candidate.pairing_id !== pairingId));
  }, [accessToken]);

  const prepareLocalHost = useCallback(async (name: string): Promise<"approval" | "activated"> => {
    const normalized = name.trim();
    if (!normalized) throw new Error("Enter a name for this computer");
    if (normalized.length > 80) throw new Error("Host names can contain at most 80 characters");
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
      throw new Error("A Forge host can only be activated from Forge Desktop");
    }
    const { invoke } = await import("@tauri-apps/api/core");
    if (await invoke<boolean>("forge_anywhere_host_enrolled")) {
      await invoke("activate_forge_anywhere_host", { name: normalized });
      await refresh();
      return "activated";
    }
    const current = credentialsRef.current;
    if (!current) throw new Error("Forge Anywhere is not signed in");
    const keys = generatePendingKeys();
    const request = {
      version: 1 as const,
      device_name: normalized,
      signing_public_key: base64Url(keys.signingPublicKey),
      exchange_public_key: base64Url(keys.exchangePublicKey),
    };
    const created = await createPairing(current.serviceUrl ?? SERVICE_URL, request);
    const challenge = parsePairingChallenge(created.challenge, current.serviceUrl ?? SERVICE_URL);
    const token = await accessToken();
    const details = await pairingDetails(current.serviceUrl ?? SERVICE_URL, token, challenge);
    setPendingLocalHost({
      name: normalized,
      keys,
      created,
      challenge,
      details,
      safetyCode: pairingSafetyCode(challenge, request.signing_public_key, current.accountIdHex),
    });
    return "approval";
  }, [accessToken, refresh]);

  const confirmLocalHost = useCallback(async () => {
    const host = pendingLocalHost;
    const current = credentialsRef.current;
    if (!host || !current) throw new Error("Local host approval expired. Start again.");
    if (Date.now() >= host.created.expires_at_ms) throw new Error("Local host approval expired. Start again.");
    const serviceUrl = current.serviceUrl ?? SERVICE_URL;
    const token = await accessToken();
    let approval: ReturnType<typeof preparePairingApproval> | null = null;
    mutationQueue.current = mutationQueue.current.catch(() => undefined).then(async () => {
      const latest = credentialsRef.current;
      if (!latest) throw new Error("Forge Anywhere is not signed in");
      const sequence = BigInt(latest.nextSequence);
      approval = preparePairingApproval(latest, host.challenge, host.details, sequence);
      await persistCredentials({ ...latest, nextSequence: (sequence + 1n).toString() });
    });
    await mutationQueue.current;
    if (!approval) throw new Error("Local host approval could not be prepared");
    await submitPairingApproval(serviceUrl, token, host.created.pairing_id, approval);

    let approved: Extract<Awaited<ReturnType<typeof pollPairing>>, { status: "approved" }> | null = null;
    for (let attempt = 0; attempt < 20 && !approved; attempt += 1) {
      const result = await pollPairing(serviceUrl, host.created.pairing_id, host.created.pairing_token);
      if (result.status === "denied") throw new Error("Local host approval was denied");
      if (result.status === "approved") approved = result;
      else await new Promise((resolve) => setTimeout(resolve, 250));
    }
    if (!approved) throw new Error("Local host approval is still pending. Try again.");
    if (approved.account_id !== current.accountIdHex) throw new Error("Local host approval came from another account");
    const opened = openApprovedPairing(approved, host.keys.exchangePrivateKey);
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("install_forge_anywhere_host", {
      name: host.name,
      state: {
        version: 1,
        account_id: approved.account_id,
        github_login: current.githubLogin ?? null,
        device_id: approved.device_id,
        signing_private_key: base64Url(host.keys.signingPrivateKey),
        exchange_private_key: base64Url(host.keys.exchangePrivateKey),
        account_data_key: base64Url(opened.accountDataKey),
        key_epoch: opened.epoch,
        data_key_epochs: { [String(opened.epoch)]: base64Url(opened.accountDataKey) },
        access_token: approved.access_token,
        refresh_token: approved.refresh_token,
        access_expires_at_ms: approved.access_expires_at_ms,
        next_sequence: 0,
      },
    });
    setPendingLocalHost(null);
    await refresh();
  }, [accessToken, pendingLocalHost, persistCredentials, refresh]);

  const cancelLocalHost = useCallback(() => setPendingLocalHost(null), []);

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
        const prepared = prepareDeviceRevocation(
          latest,
          devices,
          deviceId,
          recoveryWords,
          recovery,
          undefined,
          latest.serviceUrl ?? SERVICE_URL,
        );
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
    await enrollmentStore.clear();
    await clearAnywherePushState().catch(() => {
      // The service-side logout already revoked this device's subscriptions.
    });
    credentialsRef.current = null;
    setPushStatus("unsubscribed");
    setCredentials(null); setAccount(null); setSubscription(null); setHosts([]); setDevices([]); setPasskeys([]); setRemoteJobs([]); setFlow(null); setFlowExpiresAtMs(null); setPending(null); setRecoverySetup(null); setClaimantPairing(null); setPendingApprovalDetails([]); setPendingLocalHost(null); setApprovalError(null); setError(null);
    if (current) await clearAnywhereHostCache(current.accountIdHex).catch(() => {
      // Protected credentials are already cleared; stale metadata can be overwritten next login.
    });
    await syncAnywhereHosts([]);
    setPhase("signed_out");
  }, [syncAnywhereHosts]);

  const value: AnywhereContextValue = {
    phase, credentials, account, subscription, hosts, devices, flow, passkeys,
    recoveryWords: recoverySetup?.words ?? null,
    recoveryKit: recoverySetup
      ? createRecoveryKitV2(recoverySetup.words, SERVICE_URL, recoverySetup.auth.account_id)
      : null,
    recoverySample: [2, 6, 8],
    claimantApproval: claimantPairing ? {
      expiresAtMs: claimantPairing.created.expires_at_ms,
      safetyCode: claimantPairing.safetyCode,
      deviceName: deviceName(),
      githubLogin: claimantPairing.login.auth.github_login,
    } : null,
    localHostApproval: pendingLocalHost ? {
      name: pendingLocalHost.name,
      expiresAtMs: pendingLocalHost.created.expires_at_ms,
      safetyCode: pendingLocalHost.safetyCode,
    } : null,
    pendingApprovals: pendingApprovalDetails.map((details) => ({
      id: details.pairing_id,
      deviceId: details.device_id,
      deviceName: safeDeviceName(details.device_name),
      expiresAtMs: details.expires_at_ms,
      safetyCode: credentials
        ? pairingSafetyCode(challengeFromDetails(details, credentials.serviceUrl ?? SERVICE_URL), details.signing_public_key, credentials.accountIdHex)
        : "",
    })),
    approvalError, error, pushStatus, remoteJobs,
    accessToken, startLogin, openLoginPage, confirmNewRecovery, recoverExisting, scheduleCleanReset, cancelCleanReset, registerPasskey: registerRecoveryPasskey, recoverWithPasskey, renamePasskey: renameRecoveryPasskey, revokePasskey: revokeRecoveryPasskey, useRecoveryInstead, restartSetup, refresh, checkout, openBillingPortal,
    revokeDevice, revokeHost, renameHost, selectHost, approvePairing, approvePendingDevice, denyPendingDevice, refreshPendingApprovals, prepareLocalHost, confirmLocalHost, cancelLocalHost, queueRemoteJob, refreshRemoteJobs,
    enablePush, disablePush, logout,
  };
  const consumersReady = anywhereConsumersReady(phase, runtimeId, registeredRuntimeId);
  return <AnywhereContext.Provider value={value}>{consumersReady ? children : null}</AnywhereContext.Provider>;
}

export function useAnywhere(): AnywhereContextValue {
  const value = useContext(AnywhereContext);
  if (!value) throw new Error("useAnywhere must be used within AnywhereProvider");
  return value;
}

function message(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Forge Anywhere could not complete the request";
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function deviceName(): string {
  if (Platform.OS === "web") return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
    ? "Forge Desktop"
    : "Forge Web";
  return Platform.OS === "ios" ? "Forge on iPhone" : "Forge on Android";
}

function safeDeviceName(value: string): string {
  const sanitized = Array.from(value).filter((character) => !/[\u0000-\u001f\u007f]/.test(character)).join("").trim().slice(0, 80);
  return sanitized || "Unnamed device";
}

function snapshotKeys(keys: PendingAnywhereKeys): EnrollmentSnapshot["keys"] {
  return {
    signingPrivate: bytesToHex(keys.signingPrivateKey),
    exchangePrivate: bytesToHex(keys.exchangePrivateKey),
    signingPublic: bytesToHex(keys.signingPublicKey),
    exchangePublic: bytesToHex(keys.exchangePublicKey),
  };
}

function restorePendingKeys(keys: EnrollmentSnapshot["keys"]): PendingAnywhereKeys {
  const restored = {
    signingPrivateKey: bytesFromHex(keys.signingPrivate),
    exchangePrivateKey: bytesFromHex(keys.exchangePrivate),
    signingPublicKey: bytesFromHex(keys.signingPublic),
    exchangePublicKey: bytesFromHex(keys.exchangePublic),
  };
  if (Object.values(restored).some((key) => key.length !== 32)) {
    throw new Error("protected Forge Anywhere enrollment keys are invalid");
  }
  return restored;
}

function snapshotPairing(pairing: PendingClaimantPairing): EnrollmentSnapshot {
  return {
    version: 1,
    kind: "awaiting_approval",
    auth: pairing.login.auth,
    keys: snapshotKeys(pairing.login.keys),
    created: pairing.created,
    challenge: pairing.challenge,
    safetyCode: pairing.safetyCode,
  };
}

function snapshotAuthorizing(
  keys: PendingAnywhereKeys,
  flow: AnywhereDeviceFlow,
  flowExpiresAtMs: number,
): EnrollmentSnapshot {
  return { version: 1, kind: "authorizing", keys: snapshotKeys(keys), flow, flowExpiresAtMs };
}

function snapshotRecovery(setup: RecoverySetup): EnrollmentSnapshot {
  return {
    version: 1,
    kind: "new_recovery",
    auth: setup.auth,
    keys: snapshotKeys(setup.keys),
    words: setup.words,
    entropy: bytesToHex(setup.entropy),
    dataKey: bytesToHex(setup.dataKey),
  };
}

function snapshotExistingRecovery(login: PendingLogin): EnrollmentSnapshot {
  return { version: 1, kind: "existing_recovery", auth: login.auth, keys: snapshotKeys(login.keys) };
}

function parseEnrollmentSnapshot(value: string): EnrollmentSnapshot {
  const parsed = JSON.parse(value) as Partial<EnrollmentSnapshot>;
  if (parsed.version !== 1
    || !["authorizing", "awaiting_approval", "new_recovery", "existing_recovery"].includes(parsed.kind ?? "")
    || (parsed.kind !== "authorizing" && !parsed.auth)
    || !parsed.keys
    || typeof parsed.keys.signingPrivate !== "string"
    || typeof parsed.keys.exchangePrivate !== "string"
    || typeof parsed.keys.signingPublic !== "string"
    || typeof parsed.keys.exchangePublic !== "string") {
    throw new Error("protected Forge Anywhere enrollment state is invalid");
  }
  return parsed as EnrollmentSnapshot;
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
