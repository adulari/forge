import AsyncStorage from "@react-native-async-storage/async-storage";

// Deciding whether the app has just updated, and which kind of update it was.
//
// "I am not sure now ever if it updated" is a real gap: an OTA is applied silently on the launch
// after it downloads, and a native build arrives through TestFlight with nothing in-app to mark it.
// Both change the code running under the user with no acknowledgement anywhere.
//
// The decision is a pure function of what is running versus what was last seen, so it is testable
// without a device — unlike everything that reads expo-updates or shows a dialog.

/** What the app knows about itself right now, and what it recorded on its last launch. */
export interface UpdateFacts {
  /** `Updates.updateId` — null in dev, or on a build that has never taken an OTA. */
  updateId: string | null;
  /** The native app version, e.g. "1.0.1". Changes only with a real build. */
  appVersion: string;
  /** Persisted from the previous launch. Null on a fresh install. */
  lastSeenUpdateId: string | null;
  lastSeenAppVersion: string | null;
}

export type UpdateKind = "ota" | "app";

export interface UpdateNotice {
  kind: UpdateKind;
  /** The version to headline the dialog with. */
  appVersion: string;
}

/**
 * `null` when there is nothing to say — which is the common case and must stay silent.
 *
 * A fresh install is deliberately NOT an update: there is no previous version to have come from, and
 * greeting a first launch with "what's new" is noise. The first launch only records where it starts.
 *
 * A native version change outranks an OTA, because an OTA almost always accompanies one (a build
 * ships, then the first OTA lands on it) and "the app updated" is the honest description of that
 * pair. Reporting both would be two dialogs for one event.
 */
export function updateNotice(facts: UpdateFacts): UpdateNotice | null {
  const firstLaunch = facts.lastSeenAppVersion === null && facts.lastSeenUpdateId === null;
  if (firstLaunch) return null;

  if (facts.lastSeenAppVersion !== null && facts.lastSeenAppVersion !== facts.appVersion) {
    return { kind: "app", appVersion: facts.appVersion };
  }

  // `updateId` is null on a build running its embedded bundle. Going from an OTA back to null is a
  // rollback, which is still a change worth marking — the code under the user did change.
  if (facts.lastSeenUpdateId !== facts.updateId) {
    return { kind: "ota", appVersion: facts.appVersion };
  }

  return null;
}

const STORAGE_KEY = "forge.lastSeenBuild.v1";

export interface SeenBuild {
  updateId: string | null;
  appVersion: string | null;
}

/** Never throws: a storage failure must not stop the app opening, only the notice from firing. */
export async function loadLastSeenBuild(): Promise<SeenBuild> {
  try {
    const raw = await AsyncStorage.getItem(STORAGE_KEY);
    if (!raw) return { updateId: null, appVersion: null };
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return { updateId: null, appVersion: null };
    const record = parsed as Partial<SeenBuild>;
    return {
      updateId: typeof record.updateId === "string" ? record.updateId : null,
      appVersion: typeof record.appVersion === "string" ? record.appVersion : null,
    };
  } catch {
    return { updateId: null, appVersion: null };
  }
}

export async function rememberBuild(build: { updateId: string | null; appVersion: string }): Promise<void> {
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(build));
  } catch {
    // A notice repeated once beats a launch that fails on a write.
  }
}
