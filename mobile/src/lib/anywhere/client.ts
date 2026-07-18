// Forge Anywhere client contract. `MockAnywhereClient` (mockClient.ts) is the only
// implementation today; this interface exists so a real HTTP/WS client can drop in
// later without touching store.tsx or any screen.
import type {
  AnywhereAccount,
  AnywhereDevice,
  AnywhereHost,
  DeviceCodeAuth,
  HandoffPlan,
  HandoffProgress,
  PairChallenge,
  RemoteJob,
  RemoteJobSpec,
  ReplayShare,
  RotationStep,
  ShareExpiry,
  ShareFetchResult,
  StorageInfo,
  TransportPreference,
} from "./types";

export interface CleanupPreview {
  reclaimableBytes: number;
}

export interface AnywhereClient {
  // Account
  getAccount(): Promise<AnywhereAccount | null>;
  signInStart(): Promise<DeviceCodeAuth>;
  signInPoll(): Promise<DeviceCodeAuth>;
  signOut(): Promise<void>;
  deleteAccount(): Promise<void>;
  exportAccountData(): Promise<string>;

  // Hosts
  listHosts(): Promise<AnywhereHost[]>;
  renameHost(id: string, name: string): Promise<void>;
  disableHost(id: string): Promise<void>;
  revokeHost(id: string): Promise<void>;
  setHostTransportPreference(id: string, pref: TransportPreference): Promise<void>;

  // Devices / pairing
  listDevices(): Promise<AnywhereDevice[]>;
  startPair(codeOrScan: string): Promise<PairChallenge>;
  approvePair(id: string): Promise<void>;
  rejectPair(id: string): Promise<void>;
  /** Atomic: revoke tokens & host grants -> new key epoch -> rewrap -> commit. Requires the recovery phrase. */
  revokeDeviceAndRotate(id: string, phrase: string, onStep?: (step: RotationStep) => void): Promise<void>;

  // Remote jobs (queued jobs are immutable — cancel or replace only)
  listJobs(): Promise<RemoteJob[]>;
  queueJob(spec: RemoteJobSpec): Promise<RemoteJob>;
  cancelJob(id: string): Promise<void>;
  requeueJob(id: string): Promise<void>;

  // Replay shares
  listShares(sessionId: string): Promise<ReplayShare[]>;
  createShare(sessionId: string, expiry: ShareExpiry): Promise<ReplayShare>;
  revokeShare(id: string): Promise<void>;

  // Storage
  getStorage(): Promise<StorageInfo>;
  cleanupPreview(): Promise<CleanupPreview>;

  // Handoff
  handoffPreflight(sessionId: string): Promise<HandoffPlan>;
  handoffStart(sessionId: string, destHostId: string, onStage?: (update: HandoffProgress) => void): Promise<void>;

  // Replay share viewer
  fetchShare(shareId: string): Promise<ShareFetchResult>;
}
