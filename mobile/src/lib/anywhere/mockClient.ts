// In-memory mock of AnywhereClient — seeded to mirror the design comp's rows
// (mobile.dc.html "AW State Variants", lines 1283-1360) so every Anywhere screen is
// navigable end-to-end before the real relay backend exists. Mutations actually mutate
// this instance's state; nothing is faked at the UI layer.
import type { AnywhereClient, CleanupPreview } from "./client";
import {
  type AnywhereAccount,
  type AnywhereDevice,
  type AnywhereHost,
  type BlockedFile,
  type DeviceCodeAuth,
  type HandoffPlan,
  type HandoffProgress,
  type HandoffStage,
  type PairChallenge,
  type RemoteJob,
  type RemoteJobSpec,
  type ReplayShare,
  type RotationStep,
  type ShareExpiry,
  type ShareFetchResult,
  type StorageInfo,
  type TransportPreference,
} from "./types";

const GB = 1024 ** 3;
const MB = 1024 ** 2;
const DEVICE_CODE = "FQZT-XKDD";

function delay<T>(value: T, minMs = 100, maxMs = 400): Promise<T> {
  const ms = minMs + Math.random() * (maxMs - minMs);
  return new Promise((resolve) => setTimeout(() => resolve(value), ms));
}

function minutesAgo(min: number): number {
  return Date.now() - min * 60_000;
}

function daysAgo(days: number): number {
  return Date.now() - days * 24 * 60 * 60_000;
}

let idCounter = 0;
function nextId(prefix: string): string {
  idCounter += 1;
  return `${prefix}_${idCounter}`;
}

function mockFingerprint(): string {
  const hex = () => Math.floor(Math.random() * 0xff).toString(16).padStart(2, "0");
  return `SHA256:${Array.from({ length: 4 }, hex).join("")}…${Array.from({ length: 2 }, hex).join("")}`;
}

function seedAccount(): AnywhereAccount {
  return {
    githubLogin: "mkramer",
    entitlement: "trial",
    trialDaysLeft: 9,
    relayConnected: true,
    lastSyncAt: minutesAgo(2),
    storage: { usedBytes: Math.round(1.2 * GB), quotaBytes: 5 * GB, state: "ok" },
  };
}

function seedHosts(): AnywhereHost[] {
  return [
    {
      id: "host_atlas",
      name: "atlas",
      fingerprint: "SHA256:7f3a2c…c19e",
      connectorVersion: "0.42.1",
      heartbeatAgeSec: 8,
      state: { kind: "online", activity: "idle" },
      reachableVia: ["direct-lan", "anywhere-relay"],
      transportPreference: "auto",
    },
    {
      id: "host_forge_mini",
      name: "forge-mini",
      fingerprint: "SHA256:9b12f4…4a0d",
      connectorVersion: "0.42.1",
      heartbeatAgeSec: 3,
      state: { kind: "online", activity: "busy", sessionCount: 2 },
      reachableVia: ["direct-lan", "anywhere-relay"],
      transportPreference: "auto",
    },
    {
      id: "host_linux_box",
      name: "linux-box",
      fingerprint: "SHA256:c04ed1…7712",
      connectorVersion: "0.41.0",
      heartbeatAgeSec: 26 * 60,
      state: { kind: "stale", lastSeenAt: minutesAgo(26) },
      reachableVia: ["anywhere-relay"],
      transportPreference: "auto",
    },
    {
      id: "host_old_imac",
      name: "old-imac",
      fingerprint: "SHA256:1de8a0…2295",
      connectorVersion: "0.38.2",
      heartbeatAgeSec: 90 * 24 * 60 * 60,
      state: { kind: "revoked" },
      reachableVia: [],
      transportPreference: "direct",
    },
  ];
}

function seedDevices(): AnywhereDevice[] {
  return [
    {
      id: "dev_iphone",
      name: "iPhone 16 Pro",
      kind: "phone",
      fingerprint: "SHA256:4f21a0…a03c",
      enrolledAt: daysAgo(46),
      lastSeenAt: Date.now(),
      isThisDevice: true,
    },
    {
      id: "dev_macbook",
      name: "MacBook Pro",
      kind: "laptop",
      fingerprint: "SHA256:2ab1…88fe",
      enrolledAt: daysAgo(60),
      lastSeenAt: minutesAgo(4),
      isThisDevice: false,
    },
    {
      id: "dev_ipad",
      name: "iPad Pro",
      kind: "tablet",
      fingerprint: "SHA256:88c0e2…f13a",
      enrolledAt: daysAgo(16),
      lastSeenAt: daysAgo(2),
      isThisDevice: false,
    },
  ];
}

function seedJobs(): RemoteJob[] {
  return [
    {
      id: "job_1",
      hostId: "host_atlas",
      hostName: "atlas",
      sessionTitle: "Refactor auth flow",
      state: "running-on-host",
      createdAt: minutesAgo(12),
      updatedAt: minutesAgo(1),
    },
    {
      id: "job_2",
      hostId: "host_forge_mini",
      hostName: "forge-mini",
      sessionTitle: "Add retry backoff to sync",
      state: "uploaded-sealed",
      createdAt: minutesAgo(40),
      updatedAt: minutesAgo(38),
    },
    {
      id: "job_3",
      hostId: "host_linux_box",
      hostName: "linux-box",
      sessionTitle: "Fix flaky CI job",
      state: "waiting-for-host",
      createdAt: minutesAgo(90),
      updatedAt: minutesAgo(90),
    },
  ];
}

function seedShares(): ReplayShare[] {
  return [
    {
      id: "share_1",
      sessionId: "s1",
      url: "https://forge.dev/replay/tok_1#k=af92e1",
      expiry: "7d",
      createdAt: daysAgo(1),
      expiresAt: daysAgo(1) + 7 * 24 * 60 * 60_000,
      state: "active",
    },
    {
      id: "share_2",
      sessionId: "s1",
      url: "https://forge.dev/replay/tok_2#k=0c41bb",
      expiry: "24h",
      createdAt: daysAgo(3),
      expiresAt: daysAgo(2),
      state: "expired",
    },
  ];
}

const HANDOFF_STAGES: HandoffStage[] = [
  "scanning",
  "packaging",
  "uploading",
  "waiting-for-destination",
  "applying",
  "awaiting-ack",
  "complete",
];

const ROTATION_STEPS: RotationStep[] = ["revoking-tokens", "creating-key-epoch", "wrapping-keys", "committing", "done"];

export class MockAnywhereClient implements AnywhereClient {
  // Default signed-out — store.tsx flips this on after a successful device-code poll.
  private account: AnywhereAccount | null = null;
  private hosts: AnywhereHost[] = seedHosts();
  private devices: AnywhereDevice[] = seedDevices();
  private jobs: RemoteJob[] = seedJobs();
  private shares: ReplayShare[] = seedShares();
  private pairChallenges = new Map<string, PairChallenge>();
  private deviceCodePolls = 0;

  async getAccount(): Promise<AnywhereAccount | null> {
    return delay(this.account);
  }

  async signInStart(): Promise<DeviceCodeAuth> {
    this.deviceCodePolls = 0;
    return delay({ code: DEVICE_CODE, verifyUrl: "https://github.com/login/device", expiresInSec: 15 * 60, state: "waiting" });
  }

  async signInPoll(): Promise<DeviceCodeAuth> {
    this.deviceCodePolls += 1;
    if (this.deviceCodePolls < 3) {
      return delay({ code: DEVICE_CODE, verifyUrl: "https://github.com/login/device", expiresInSec: 15 * 60, state: "waiting" });
    }
    this.account = seedAccount();
    return delay({ code: DEVICE_CODE, verifyUrl: "https://github.com/login/device", expiresInSec: 15 * 60, state: "approved" });
  }

  async signOut(): Promise<void> {
    this.account = null;
    await delay(undefined);
  }

  async deleteAccount(): Promise<void> {
    this.account = null;
    await delay(undefined);
  }

  async exportAccountData(): Promise<string> {
    return delay(JSON.stringify({ account: this.account, hosts: this.hosts, devices: this.devices }, null, 2));
  }

  async listHosts(): Promise<AnywhereHost[]> {
    return delay([...this.hosts]);
  }

  async renameHost(id: string, name: string): Promise<void> {
    const host = this.hosts.find((h) => h.id === id);
    if (host) host.name = name;
    await delay(undefined);
  }

  async disableHost(id: string): Promise<void> {
    const host = this.hosts.find((h) => h.id === id);
    if (host) host.state = { kind: "disabled" };
    await delay(undefined);
  }

  async revokeHost(id: string): Promise<void> {
    const host = this.hosts.find((h) => h.id === id);
    if (host) host.state = { kind: "revoked" };
    await delay(undefined);
  }

  async setHostTransportPreference(id: string, pref: TransportPreference): Promise<void> {
    const host = this.hosts.find((h) => h.id === id);
    if (host) host.transportPreference = pref;
    await delay(undefined);
  }

  async listDevices(): Promise<AnywhereDevice[]> {
    return delay([...this.devices]);
  }

  async startPair(codeOrScan: string): Promise<PairChallenge> {
    const challenge: PairChallenge = {
      id: nextId("pair"),
      code: codeOrScan,
      state: "pending",
      createdAt: Date.now(),
      expiresAt: Date.now() + 10 * 60_000,
      account: this.account?.githubLogin ?? "unknown",
      deviceName: "New device",
      deviceKind: "phone",
      fingerprint: mockFingerprint(),
      grants: ["control", "sync"],
    };
    this.pairChallenges.set(challenge.id, challenge);
    return delay(challenge);
  }

  async approvePair(id: string): Promise<void> {
    const challenge = this.pairChallenges.get(id);
    if (challenge) {
      challenge.state = "approved";
      this.devices.push({
        id: nextId("dev"),
        name: challenge.deviceName,
        kind: challenge.deviceKind,
        fingerprint: challenge.fingerprint,
        enrolledAt: Date.now(),
        lastSeenAt: Date.now(),
        isThisDevice: false,
      });
    }
    await delay(undefined);
  }

  async rejectPair(id: string): Promise<void> {
    const challenge = this.pairChallenges.get(id);
    if (challenge) challenge.state = "rejected";
    await delay(undefined);
  }

  async revokeDeviceAndRotate(id: string, phrase: string, onStep?: (step: RotationStep) => void): Promise<void> {
    if (phrase.trim().split(/\s+/).filter(Boolean).length < 24) {
      onStep?.("failed");
      throw new Error("Recovery phrase required to rotate keys");
    }
    for (const step of ROTATION_STEPS) {
      await delay(undefined, 150, 350);
      onStep?.(step);
    }
    this.devices = this.devices.filter((d) => d.id !== id);
  }

  async listJobs(): Promise<RemoteJob[]> {
    return delay([...this.jobs]);
  }

  async queueJob(spec: RemoteJobSpec): Promise<RemoteJob> {
    const host = this.hosts.find((h) => h.id === spec.hostId);
    const job: RemoteJob = {
      id: nextId("job"),
      hostId: spec.hostId,
      hostName: host?.name ?? "unknown host",
      sessionTitle: spec.prompt.slice(0, 60),
      state: host?.state.kind === "online" ? "running-on-host" : "waiting-for-host",
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    this.jobs = [job, ...this.jobs];
    return delay(job);
  }

  async cancelJob(id: string): Promise<void> {
    this.jobs = this.jobs.filter((j) => j.id !== id);
    await delay(undefined);
  }

  async requeueJob(id: string): Promise<void> {
    const job = this.jobs.find((j) => j.id === id);
    if (job) {
      job.state = "waiting-for-host";
      job.updatedAt = Date.now();
    }
    await delay(undefined);
  }

  async listShares(sessionId: string): Promise<ReplayShare[]> {
    return delay(this.shares.filter((s) => s.sessionId === sessionId));
  }

  async createShare(sessionId: string, expiry: ShareExpiry): Promise<ReplayShare> {
    const ttlMs = expiry === "24h" ? 24 * 60 * 60_000 : expiry === "7d" ? 7 * 24 * 60 * 60_000 : 30 * 24 * 60 * 60_000;
    const share: ReplayShare = {
      id: nextId("share"),
      sessionId,
      url: `https://forge.dev/replay/${nextId("tok")}#k=${mockFingerprint().replace("SHA256:", "")}`,
      expiry,
      createdAt: Date.now(),
      expiresAt: Date.now() + ttlMs,
      state: "active",
    };
    this.shares = [...this.shares, share];
    return delay(share);
  }

  async revokeShare(id: string): Promise<void> {
    const share = this.shares.find((s) => s.id === id);
    if (share) share.state = "revoked";
    await delay(undefined);
  }

  async getStorage(): Promise<StorageInfo> {
    return delay(this.account?.storage ?? { usedBytes: 0, quotaBytes: 5 * GB, state: "ok" });
  }

  async cleanupPreview(): Promise<CleanupPreview> {
    return delay({ reclaimableBytes: Math.round(0.3 * GB) });
  }

  async handoffPreflight(_sessionId: string): Promise<HandoffPlan> {
    const blockedFiles: BlockedFile[] = [
      { path: ".env", reason: "detected secret" },
      { path: "target/", reason: "ignored build output" },
      { path: "assets/demo.mov", reason: "31 MB > 25 MB cap" },
    ];
    return delay({
      checkpoint: "c-41",
      baseCommit: "9d2f31c",
      fileCount: 214,
      capsuleBytes: Math.round(18.2 * MB),
      blockedFiles,
    });
  }

  async handoffStart(
    _sessionId: string,
    _destHostId: string,
    onStage?: (update: HandoffProgress) => void,
  ): Promise<void> {
    for (const stage of HANDOFF_STAGES) {
      await delay(undefined, 200, 500);
      onStage?.({ stage });
    }
  }

  async fetchShare(shareId: string): Promise<ShareFetchResult> {
    const share = this.shares.find((s) => s.id === shareId);
    if (!share) return delay({ ok: false, error: "expired" });
    if (share.state === "revoked") return delay({ ok: false, error: "revoked" });
    if (share.state === "expired" || share.expiresAt < Date.now()) return delay({ ok: false, error: "expired" });
    return delay({
      ok: true,
      payload: {
        sessionTitle: `Session ${share.sessionId}`,
        transcript: "(mock transcript)",
        createdAt: share.createdAt,
      },
    });
  }
}
