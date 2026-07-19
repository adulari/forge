// Forge Anywhere — foundation types for the optional managed encrypted-relay transport.
// Mirrors the state machines in the design comp (mobile.dc.html "AW State Variants",
// lines 1283-1360): entitlement lifecycle, host/device state, sync glyphs, handoff
// stages, remote jobs, replay shares, storage, session transport, and sign-in flows.
// Discriminated unions are used everywhere a state carries different payloads so
// consumers get exhaustive-switch checking instead of optional-field guessing.

// ---------------------------------------------------------------------------
// Entitlement
// ---------------------------------------------------------------------------

// NOT STARTED -> TRIAL (14d, starts on first host connect, no card) -> ACTIVE
// (Paddle) -> GRACE (7d, paid payment-failure only; trial expiry skips grace) ->
// READ-ONLY (30d: download/restore/delete/export/billing only) -> SUSPENDED
// (billing/export/delete only, until the 90-day retention deadline).
export type EntitlementState =
  | "not-started"
  | "trial"
  | "active"
  | "grace"
  | "read-only"
  | "suspended"
  // Paddle checkout succeeded but the entitlement webhook hasn't landed yet.
  | "webhook-pending";

export type BillingPlan = "monthly" | "yearly"; // EUR10/mo or EUR79/yr via Paddle

export type StorageState = "calculating" | "ok" | "nearly-full" | "full" | "stale-figure";

export interface StorageInfo {
  usedBytes: number;
  quotaBytes: number;
  state: StorageState;
}

export type RetentionKind = "capsules" | "superseded-revisions" | "tombstones" | "shares" | "post-subscription";

/** One row of the storage retention table. `windowDays: null` means "until expiry" (shares). */
export interface RetentionRow {
  kind: RetentionKind;
  label: string;
  windowDays: number | null;
}

export interface AnywhereAccount {
  githubLogin: string;
  entitlement: EntitlementState;
  /** Days remaining in the trial window. Present only while `entitlement === "trial"`. */
  trialDaysLeft?: number;
  /** Days remaining in the grace window. Present only while `entitlement === "grace"`. */
  graceDaysLeft?: number;
  /** Days remaining before read-only access ends. Present only while `entitlement === "read-only"`. */
  readOnlyDaysLeft?: number;
  /** Days until the 90-day retention deadline deletes data. Present only while `entitlement === "suspended"`. */
  deletesInDays?: number;
  plan?: BillingPlan;
  relayConnected: boolean;
  lastSyncAt: number | null;
  storage: StorageInfo;
}

/** Business rule from the design spec: at most 3 active hosts per account. */
export const MAX_ACTIVE_HOSTS = 3;

// ---------------------------------------------------------------------------
// Hosts
// ---------------------------------------------------------------------------

export type HostReachability = "direct-lan" | "anywhere-relay";

export type HostState =
  | { kind: "online"; activity: "idle" }
  | { kind: "online"; activity: "busy"; sessionCount: number }
  | { kind: "connecting" }
  | { kind: "stale"; lastSeenAt: number }
  | { kind: "offline"; lastHeartbeatAt: number }
  | { kind: "disabled" }
  | { kind: "revoked" }
  // Connector version too old to speak the current protocol.
  | { kind: "update-required"; connectorVersion: string };

export interface AnywhereHost {
  id: string;
  /** Renamable. */
  name: string;
  /** SHA256 identity fingerprint — never changes, even across renames/reconnects. */
  fingerprint: string;
  connectorVersion: string;
  heartbeatAgeSec: number;
  state: HostState;
  reachableVia: HostReachability[];
  /** Per-host default transport for new sessions. */
  transportPreference: TransportPreference;
}

// ---------------------------------------------------------------------------
// Devices (paired controllers)
// ---------------------------------------------------------------------------

export type DeviceKind = "phone" | "tablet" | "laptop";

export interface AnywhereDevice {
  id: string;
  name: string;
  kind: DeviceKind;
  fingerprint: string;
  enrolledAt: number;
  lastSeenAt: number;
  isThisDevice: boolean;
}

// ---------------------------------------------------------------------------
// Pairing (QR / paste challenge, 10-min expiry)
// ---------------------------------------------------------------------------

export type PairChallengeState =
  | "pending"
  | "approved"
  | "rejected"
  | "expired"
  | "already-used"
  | "wrong-account"
  | "malformed"
  | "camera-denied";

export interface PairChallenge {
  id: string;
  code: string;
  state: PairChallengeState;
  createdAt: number;
  expiresAt: number;
  account: string;
  deviceName: string;
  deviceKind: DeviceKind;
  fingerprint: string;
  grants: string[];
}

// Device revoke = key rotation: atomic, requires the recovery phrase.
export type RotationStep =
  | "revoking-tokens"
  | "creating-key-epoch"
  | "wrapping-keys"
  | "committing"
  | "done"
  | "failed";

// ---------------------------------------------------------------------------
// Sync status (mono glyph + color, per-row)
// ---------------------------------------------------------------------------

export type SyncStatus =
  | { kind: "synced"; syncedAt: number; offlineAvailable: boolean }
  | { kind: "uploading"; recordCount: number }
  | { kind: "downloading"; bytes: number }
  | { kind: "offline-cache"; cachedAt: number }
  | { kind: "retrying"; attempt: number; lastSuccessAt: number | null }
  | { kind: "conflict"; bothKept: boolean }
  | { kind: "over-quota" }
  | { kind: "key-epoch-required" }
  | { kind: "read-only" };

// ---------------------------------------------------------------------------
// Handoff (session capsule move between hosts)
// ---------------------------------------------------------------------------

export type HandoffStage =
  | "eligible"
  | "waiting-for-checkpoint"
  | "scanning"
  | "blocked"
  | "packaging"
  | "uploading"
  | "waiting-for-destination"
  | "applying"
  | "awaiting-ack"
  | "complete"
  | "rolled-back"
  | "expired";

export interface BlockedFile {
  path: string;
  reason: string;
}

export interface HandoffPlan {
  checkpoint: string;
  baseCommit: string;
  fileCount: number;
  capsuleBytes: number;
  blockedFiles: BlockedFile[];
}

/** Progress callback payload for `AnywhereClient.handoffStart`. */
export interface HandoffProgress {
  stage: HandoffStage;
  blockedFiles?: BlockedFile[];
}

// ---------------------------------------------------------------------------
// Remote jobs — queued jobs are immutable (cancel or replace only, never edit).
// ---------------------------------------------------------------------------

export type JobState =
  | "running-on-host"
  | "waiting-for-host"
  | "uploaded-sealed"
  | "queued-locally-offline"
  | "completed"
  | "failed"
  | "expired-after-7d-unclaimed"
  | "blocked-read-only-plan";

export interface RemoteJob {
  id: string;
  hostId: string;
  hostName: string;
  sessionTitle: string;
  state: JobState;
  createdAt: number;
  updatedAt: number;
}

export interface RemoteJobSpec {
  hostId: string;
  prompt: string;
}

// ---------------------------------------------------------------------------
// Replay shares — read-only e2e-encrypted links, key in the URL fragment.
// ---------------------------------------------------------------------------

export type ShareExpiry = "24h" | "7d" | "30d";
export type ShareState = "active" | "expired" | "revoked";

export interface ReplayShare {
  id: string;
  sessionId: string;
  /** Decryption key lives in the URL fragment (`#k=...`) — never sent to the server. */
  url: string;
  expiry: ShareExpiry;
  createdAt: number;
  expiresAt: number;
  state: ShareState;
}

export type ShareRetrievalError =
  | "key-fragment-missing"
  | "corrupted"
  | "expired"
  | "revoked"
  | "service-unavailable";

export interface ShareViewerPayload {
  sessionTitle: string;
  transcript: string;
  createdAt: number;
}

export type ShareFetchResult =
  | { ok: true; payload: ShareViewerPayload }
  | { ok: false; error: ShareRetrievalError };

// ---------------------------------------------------------------------------
// Session transport
// ---------------------------------------------------------------------------

export type SessionTransport = "direct" | "anywhere";
export type TransportPreference = "auto" | "direct" | "anywhere";

export interface SessionTransportInfo {
  hostName: string;
  transport: SessionTransport;
  strip?: StripCondition;
}

/** The 7 session status-strip conditions from the design spec. */
export type StripCondition =
  | { kind: "reconnecting-via-relay"; retryInSec: number }
  | { kind: "host-asleep-input-queued" }
  | { kind: "relay-unreachable-direct-paired" }
  | { kind: "relay-unreachable-not-paired" }
  | { kind: "read-only-controlling-elsewhere" }
  | { kind: "plan-read-only" }
  | { kind: "capsule-upload"; progressPct: number; mbPerSec: number };

// ---------------------------------------------------------------------------
// Sign-in (GitHub device-code flow) + recovery
// ---------------------------------------------------------------------------

export type DeviceCodeAuthState = "waiting" | "approved" | "expired" | "denied" | "network-failed";

export interface DeviceCodeAuth {
  code: string;
  verifyUrl: string;
  expiresInSec: number;
  state: DeviceCodeAuthState;
}

export type RecoveryEntryState =
  | { kind: "entering" }
  | { kind: "wrong-checksum"; wordIndex: number }
  | { kind: "no-wrapped-key-for-epoch" }
  | { kind: "device-revoked" }
  | { kind: "terminal-unrecoverable" }
  | { kind: "recovered" };
