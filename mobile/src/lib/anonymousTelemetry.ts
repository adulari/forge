// Anonymous product counters for the mobile and desktop shells. The event contract is closed on
// purpose: no caller can attach user content or a stable identifier. Local period markers make
// event counts equal active installations while every wire event uses one shared distinct_id.
import AsyncStorage from "@react-native-async-storage/async-storage";
import Constants from "expo-constants";
import { AppState, Platform } from "react-native";

import { isTauri } from "./platform";

const STATE_KEY = "forge.anonymousTelemetry.state.v1";
const ENABLED_KEY = "forge.anonymousTelemetry.enabled";
const NOTICE_KEY = "forge.anonymousTelemetry.notice.v1";
const DISTINCT_ID = "forge-anonymous";
const DEFAULT_HOST = "https://eu.i.posthog.com";

type EventName =
  | "forge_installed"
  | "forge_active_month"
  | "forge_active_week"
  | "forge_active_day"
  | "forge_active_window"
  | "forge_activated"
  | "forge_app_error";

export type AnonymousAppErrorCode = "react_render";

const ALLOWED_EVENTS = new Set<EventName>([
  "forge_installed",
  "forge_active_month",
  "forge_active_week",
  "forge_active_day",
  "forge_active_window",
  "forge_activated",
  "forge_app_error",
]);

interface PendingEvent {
  event: EventName;
  period: string;
  localId: string;
  errorCode?: AnonymousAppErrorCode;
}

interface TelemetryState {
  installed: boolean;
  activated: boolean;
  day: string;
  week: string;
  month: string;
  window: string;
  pending: PendingEvent[];
}

const EMPTY_STATE: TelemetryState = {
  installed: false,
  activated: false,
  day: "",
  week: "",
  month: "",
  window: "",
  pending: [],
};

let started = false;
let preferenceGeneration = 0;
let activeController: AbortController | null = null;
let recording: Promise<void> | null = null;
let activationRequested = false;
const pendingAppErrors: AnonymousAppErrorCode[] = [];

function apiKey(): string | undefined {
  const key = process.env.EXPO_PUBLIC_POSTHOG_KEY;
  return key?.trim() || undefined;
}

function host(): string {
  return (process.env.EXPO_PUBLIC_POSTHOG_HOST?.trim() || DEFAULT_HOST).replace(/\/$/, "");
}

function isFalse(value: string | undefined): boolean {
  return value != null && ["0", "false", "no", "off"].includes(value.trim().toLowerCase());
}

function doNotTrack(): boolean {
  if (isFalse(process.env.EXPO_PUBLIC_FORGE_TELEMETRY)) return true;
  if (__DEV__ && process.env.EXPO_PUBLIC_FORGE_TELEMETRY_FORCE !== "1") return true;
  return typeof navigator !== "undefined" && navigator.doNotTrack === "1";
}

export async function isAnonymousTelemetryEnabled(): Promise<boolean> {
  if (doNotTrack()) return false;
  const stored = await AsyncStorage.getItem(ENABLED_KEY);
  return stored !== "false";
}

export async function setAnonymousTelemetryEnabled(enabled: boolean): Promise<void> {
  preferenceGeneration += 1;
  await AsyncStorage.setItem(ENABLED_KEY, enabled ? "true" : "false");
  if (!enabled) {
    activeController?.abort();
    await AsyncStorage.removeItem(STATE_KEY);
  }
  if (enabled) {
    const prior = recording;
    if (prior) {
      void prior.finally(() => scheduleActivity());
    } else {
      void scheduleActivity();
    }
  }
}

export async function shouldShowAnonymousTelemetryNotice(): Promise<boolean> {
  if (!apiKey() || !(await isAnonymousTelemetryEnabled())) return false;
  const shown = await AsyncStorage.getItem(NOTICE_KEY);
  if (shown === "true") return false;
  await AsyncStorage.setItem(NOTICE_KEY, "true");
  return true;
}

export function startAnonymousTelemetry(): () => void {
  if (started) return () => undefined;
  started = true;
  void scheduleActivity();
  const subscription = AppState.addEventListener("change", (state) => {
    if (state === "active") void scheduleActivity();
  });
  const heartbeat = setInterval(() => {
    if (AppState.currentState === "active") void scheduleActivity();
  }, 30 * 60 * 1_000);
  return () => {
    subscription.remove();
    clearInterval(heartbeat);
    started = false;
  };
}

/** Mark the first successful server pairing as product activation, without identifying it. */
export function markAnonymousTelemetryActivated(): void {
  activationRequested = true;
  void scheduleActivity();
}

/**
 * Record a bounded app failure category without accepting the Error, message, stack, route, or
 * other user-controlled data.
 */
export function markAnonymousTelemetryAppError(code: AnonymousAppErrorCode): void {
  pendingAppErrors.push(code);
  void scheduleActivity();
}

function scheduleActivity(): Promise<void> {
  if (recording) return recording;
  recording = recordActivity().finally(() => {
    recording = null;
    // An error may have arrived while storage or the network was in flight.
    if (pendingAppErrors.length > 0) void scheduleActivity();
  });
  return recording;
}

function utcPeriods(date: Date) {
  const day = date.toISOString().slice(0, 10);
  const month = day.slice(0, 7);
  const dayDate = new Date(`${day}T00:00:00Z`);
  const weekday = dayDate.getUTCDay() || 7;
  dayDate.setUTCDate(dayDate.getUTCDate() + 4 - weekday);
  const yearStart = new Date(Date.UTC(dayDate.getUTCFullYear(), 0, 1));
  const weekNumber = Math.ceil(((dayDate.getTime() - yearStart.getTime()) / 86_400_000 + 1) / 7);
  const week = `${dayDate.getUTCFullYear()}-W${String(weekNumber).padStart(2, "0")}`;
  const window = String(Math.floor(date.getTime() / 1000 / (30 * 60)));
  return { day, week, month, window };
}

function queue(state: TelemetryState, event: EventName, period: string) {
  if (!state.pending.some((item) => item.event === event && item.period === period)) {
    state.pending.push({ event, period, localId: `${Date.now()}-${Math.random()}` });
  }
}

function queueAppError(
  state: TelemetryState,
  code: AnonymousAppErrorCode,
  period: string,
): void {
  state.pending.push({
    event: "forge_app_error",
    period,
    localId: `${Date.now()}-${Math.random()}`,
    errorCode: code,
  });
}

async function recordActivity(): Promise<void> {
  const generation = preferenceGeneration;
  const key = apiKey();
  if (!key || !(await isAnonymousTelemetryEnabled())) {
    pendingAppErrors.length = 0;
    return;
  }
  const appErrors = pendingAppErrors.splice(0);

  const raw = await AsyncStorage.getItem(STATE_KEY);
  let state: TelemetryState = { ...EMPTY_STATE, pending: [] };
  if (raw) {
    try {
      state = { ...state, ...(JSON.parse(raw) as Partial<TelemetryState>) };
    } catch {
      // A corrupt analytics marker is disposable and contains no user data.
    }
  }
  if (generation !== preferenceGeneration || !(await isAnonymousTelemetryEnabled())) return;

  const periods = utcPeriods(new Date());
  for (const code of appErrors) queueAppError(state, code, periods.window);
  if (!state.installed) queue(state, "forge_installed", "once");
  state.installed = true;
  if (activationRequested && !state.activated) {
    state.activated = true;
    queue(state, "forge_activated", "once");
  }
  for (const [marker, event] of [
    ["month", "forge_active_month"],
    ["week", "forge_active_week"],
    ["day", "forge_active_day"],
    ["window", "forge_active_window"],
  ] as const) {
    if (state[marker] !== periods[marker]) {
      state[marker] = periods[marker];
      queue(state, event, periods[marker]);
    }
  }
  await AsyncStorage.setItem(STATE_KEY, JSON.stringify(state));
  if (state.pending.length === 0) return;

  if (generation !== preferenceGeneration || !(await isAnonymousTelemetryEnabled())) return;
  const sent = state.pending.filter((item) => ALLOWED_EVENTS.has(item.event));
  if (sent.length === 0) return;
  if (await send(key, sent)) {
    const currentRaw = await AsyncStorage.getItem(STATE_KEY);
    if (!currentRaw) return;
    try {
      const current = JSON.parse(currentRaw) as TelemetryState;
      current.pending = current.pending.filter(
        (item) => !sent.some((done) => done.localId === item.localId),
      );
      await AsyncStorage.setItem(STATE_KEY, JSON.stringify(current));
    } catch {
      // Leave the state untouched; the next launch can safely retry the anonymous counters.
    }
  }
}

function surface(): "desktop" | "mobile" | "web" {
  if (isTauri) return "desktop";
  return Platform.OS === "web" ? "web" : "mobile";
}

async function send(key: string, events: PendingEvent[]): Promise<boolean> {
  const controller = new AbortController();
  activeController = controller;
  const timer = setTimeout(() => controller.abort(), 2_000);
  try {
    const response = await fetch(`${host()}/batch/`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal: controller.signal,
      body: JSON.stringify({
        api_key: key,
        batch: events.map(({ event, period, errorCode }) => ({
          event,
          properties: {
            distinct_id: DISTINCT_ID,
            $process_person_profile: false,
            $geoip_disable: true,
            surface: surface(),
            version: Constants.expoConfig?.version ?? "unknown",
            os: Platform.OS,
            distribution: isTauri ? "desktop-release" : "app",
            period,
            ...(errorCode ? { error_code: errorCode } : {}),
            schema: 1,
          },
        })),
      }),
    });
    return response.ok;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
    if (activeController === controller) activeController = null;
  }
}
