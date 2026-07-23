import { beforeEach, describe, expect, it, vi } from "vitest";

import { markAnonymousTelemetryAppError } from "./anonymousTelemetry";

const storage = vi.hoisted(() => new Map<string, string>());

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: {
    getItem: vi.fn(async (key: string) => storage.get(key) ?? null),
    setItem: vi.fn(async (key: string, value: string) => {
      storage.set(key, value);
    }),
    removeItem: vi.fn(async (key: string) => {
      storage.delete(key);
    }),
  },
}));

vi.mock("expo-constants", () => ({
  default: { expoConfig: { version: "2.8.5" } },
}));

vi.mock("react-native", () => ({
  AppState: {
    currentState: "active",
    addEventListener: vi.fn(() => ({ remove: vi.fn() })),
  },
  Platform: { OS: "ios" },
}));

vi.mock("./platform", () => ({ isTauri: false }));

describe("anonymous app error telemetry", () => {
  beforeEach(() => {
    storage.clear();
    vi.stubGlobal("__DEV__", false);
    process.env.EXPO_PUBLIC_POSTHOG_KEY = "test-project-key";
    process.env.EXPO_PUBLIC_FORGE_TELEMETRY = "1";
  });

  it("sends only the closed error code and release dimensions", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(null, { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    markAnonymousTelemetryAppError("react_render");

    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const request = fetchMock.mock.calls[0]?.[1];
    if (!request) throw new Error("telemetry request options were not captured");
    const payload = JSON.parse(String(request.body)) as {
      batch: { event: string; properties: Record<string, unknown> }[];
    };
    const errorEvent = payload.batch.find((item) => item.event === "forge_app_error");

    expect(errorEvent?.properties).toMatchObject({
      distinct_id: "forge-anonymous",
      $process_person_profile: false,
      $geoip_disable: true,
      error_code: "react_render",
      surface: "mobile",
      version: "2.8.5",
      os: "ios",
      distribution: "app",
      schema: 1,
    });
    expect(Object.keys(errorEvent?.properties ?? {}).sort()).toEqual(
      [
        "$geoip_disable",
        "$process_person_profile",
        "distinct_id",
        "distribution",
        "error_code",
        "os",
        "period",
        "schema",
        "surface",
        "version",
      ].sort(),
    );
  });
});
