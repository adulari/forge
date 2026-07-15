import { Platform } from "react-native";
import { requireNativeModule } from "expo-modules-core";

// Real module only exists on iOS (see ios/LiveActivityModule.swift) — every export here
// degrades to a safe no-op on other platforms so callers don't need their own Platform.OS guard.
type NativeLiveActivity = {
  isSupported(): Promise<boolean>;
  start(
    sessionId: string,
    title: string,
    busy: boolean,
    waiting: boolean,
    costUsd: number,
    contextTokens: number,
    contextLimit: number,
  ): Promise<{ activityId: string | null; pushToken: string | null }>;
  update(
    activityId: string,
    busy: boolean,
    waiting: boolean,
    costUsd: number,
    contextTokens: number,
    contextLimit: number,
  ): Promise<void>;
  end(activityId: string): Promise<void>;
};

export type LiveActivityPushToken = {
  sessionId: string;
  pushToken: string;
};

type LiveActivityEvents = {
  pushToken: (token: LiveActivityPushToken) => void;
};

type NativeLiveActivityModule = NativeLiveActivity & {
  addListener(
    eventName: keyof LiveActivityEvents,
    listener: LiveActivityEvents["pushToken"],
  ): { remove(): void };
};

function loadNative(): NativeLiveActivityModule | null {
  if (Platform.OS !== "ios") return null;
  try {
    return requireNativeModule<NativeLiveActivityModule>("LiveActivity");
  } catch {
    return null;
  }
}

const native = loadNative();
const emitter = native;

export function addLiveActivityPushTokenListener(
  listener: (token: LiveActivityPushToken) => void,
): { remove(): void } {
  return emitter?.addListener("pushToken", listener) ?? { remove() {} };
}

export async function isLiveActivitySupported(): Promise<boolean> {
  if (!native) return false;
  return native.isSupported();
}

/** Starts (or reuses) a Live Activity for a session's turn. Returns `null` for `pushToken` if
 * Live Activities are unsupported/disabled or no token arrived within the native module's
 * timeout — the caller should still treat `activityId` as running either way. */
export async function startLiveActivity(
  sessionId: string,
  title: string,
  busy: boolean,
  waiting: boolean,
  costUsd: number,
  contextTokens: number,
  contextLimit: number,
): Promise<{ activityId: string | null; pushToken: string | null }> {
  if (!native) return { activityId: null, pushToken: null };
  return native.start(sessionId, title, busy, waiting, costUsd, contextTokens, contextLimit);
}

export async function updateLiveActivity(
  activityId: string,
  busy: boolean,
  waiting: boolean,
  costUsd: number,
  contextTokens: number,
  contextLimit: number,
): Promise<void> {
  if (!native) return;
  await native.update(activityId, busy, waiting, costUsd, contextTokens, contextLimit);
}

export async function endLiveActivity(activityId: string): Promise<void> {
  if (!native) return;
  await native.end(activityId);
}
