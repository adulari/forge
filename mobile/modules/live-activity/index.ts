import { Platform } from "react-native";
import { requireNativeModule } from "expo-modules-core";

// Real module only exists on iOS (see ios/LiveActivityModule.swift) — every export here
// degrades to a safe no-op on other platforms so callers don't need their own Platform.OS guard.

/** Mirrors ForgeLiveActivityAttributesInput (ios/LiveActivityModule.swift). */
export type LiveActivityAttributes = {
  sessionId: string;
  title: string;
  /** Daemon base URL (embeds the auth token) — the widget's Allow/Deny intents POST to it. */
  baseUrl: string;
  agentLabel: string;
};

/** Mirrors ForgeLiveActivityStateInput (ios/LiveActivityModule.swift). */
export type LiveActivityState = {
  busy: boolean;
  waiting: boolean;
  costUsd: number;
  contextTokens: number;
  contextLimit: number;
  question?: string | null;
  promptSeq?: number | null;
  tasksDone?: number | null;
  tasksTotal?: number | null;
  /** Unix seconds of the last busy/waiting state transition (drives the elapsed timer). */
  stateSinceEpoch?: number | null;
};

type NativeLiveActivity = {
  isSupported(): Promise<boolean>;
  start(
    attributes: LiveActivityAttributes,
    state: LiveActivityState,
  ): Promise<{ activityId: string | null; pushToken: string | null }>;
  update(activityId: string, state: LiveActivityState): Promise<void>;
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
  attributes: LiveActivityAttributes,
  state: LiveActivityState,
): Promise<{ activityId: string | null; pushToken: string | null }> {
  if (!native) return { activityId: null, pushToken: null };
  return native.start(attributes, state);
}

export async function updateLiveActivity(activityId: string, state: LiveActivityState): Promise<void> {
  if (!native) return;
  await native.update(activityId, state);
}

export async function endLiveActivity(activityId: string): Promise<void> {
  if (!native) return;
  await native.end(activityId);
}
