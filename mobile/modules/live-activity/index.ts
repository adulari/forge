import { Platform } from "react-native";
import { EventEmitter, requireNativeModule } from "expo-modules-core";

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
  ): Promise<{ activityId: string | null; pushToken: string | null }>;
  update(
    activityId: string,
    busy: boolean,
    waiting: boolean,
    costUsd: number,
    contextTokens: number,
  ): Promise<void>;
  end(activityId: string): Promise<void>;
};

function loadNative(): NativeLiveActivity | null {
  if (Platform.OS !== "ios") return null;
  try {
    return requireNativeModule("LiveActivity");
  } catch {
    return null;
  }
}

const native = loadNative();
const emitter = native ? new EventEmitter(native) : null;

export type LiveActivityPushToken = {
  sessionId: string;
  pushToken: string;
};

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
): Promise<{ activityId: string | null; pushToken: string | null }> {
  if (!native) return { activityId: null, pushToken: null };
  return native.start(sessionId, title, busy, waiting, costUsd, contextTokens);
}

export async function updateLiveActivity(
  activityId: string,
  busy: boolean,
  waiting: boolean,
  costUsd: number,
  contextTokens: number,
): Promise<void> {
  if (!native) return;
  await native.update(activityId, busy, waiting, costUsd, contextTokens);
}

export async function endLiveActivity(activityId: string): Promise<void> {
  if (!native) return;
  await native.end(activityId);
}
