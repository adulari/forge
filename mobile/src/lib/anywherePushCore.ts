export type AnywherePushStatus = "unsupported" | "denied" | "unsubscribed" | "subscribed";
export type AnywherePushEnvironment = "sandbox" | "production";

export interface AnywherePushRegistration {
  subscriptionId: string;
  environment: AnywherePushEnvironment;
}

export interface AnywherePushStorage {
  load(): Promise<AnywherePushRegistration | null>;
  save(registration: AnywherePushRegistration): Promise<void>;
  clear(): Promise<void>;
}

export interface AnywherePushPlatform {
  supported(): boolean;
  permission(): Promise<"granted" | "denied" | "undetermined">;
  requestPermission(): Promise<"granted" | "denied" | "undetermined">;
  deviceToken(): Promise<string>;
  environment(): AnywherePushEnvironment;
  observeRefresh(onRefresh: () => void): () => void;
}

export interface AnywherePushApi {
  register(input: {
    platform: "apns";
    environment: AnywherePushEnvironment;
    device_token: string;
  }): Promise<{ subscription_id: string }>;
  revoke(subscriptionId: string): Promise<void>;
}

export async function anywherePushStatus(
  platform: AnywherePushPlatform,
  storage: AnywherePushStorage,
): Promise<AnywherePushStatus> {
  if (!platform.supported()) return "unsupported";
  const permission = await platform.permission();
  if (permission === "denied") return "denied";
  if (permission !== "granted") return "unsubscribed";
  return (await storage.load()) ? "subscribed" : "unsubscribed";
}

export async function enableAnywherePush(
  platform: AnywherePushPlatform,
  storage: AnywherePushStorage,
  api: AnywherePushApi,
): Promise<AnywherePushStatus> {
  if (!platform.supported()) return "unsupported";
  const existing = await platform.permission();
  if (existing === "denied") return "denied";
  const permission = existing === "granted" ? existing : await platform.requestPermission();
  if (permission !== "granted") return permission === "denied" ? "denied" : "unsubscribed";

  const deviceToken = await platform.deviceToken();
  if (!/^[0-9a-f]{64}$/.test(deviceToken)) {
    throw new Error("APNs returned an invalid device token");
  }
  const environment = platform.environment();
  const response = await api.register({ platform: "apns", environment, device_token: deviceToken });
  if (!/^[0-9a-f]{32}$/.test(response.subscription_id)) {
    throw new Error("Forge Anywhere returned an invalid push subscription");
  }
  // The APNs token is deliberately not retained on-device by this feature. Only the opaque
  // subscription identifier needed for revocation is protected in SecureStore.
  await storage.save({ subscriptionId: response.subscription_id, environment });
  return "subscribed";
}

export async function disableAnywherePush(
  platform: AnywherePushPlatform,
  storage: AnywherePushStorage,
  api: AnywherePushApi,
): Promise<AnywherePushStatus> {
  if (!platform.supported()) return "unsupported";
  const registration = await storage.load();
  if (registration) await api.revoke(registration.subscriptionId);
  await storage.clear();
  return "unsubscribed";
}

export function observeAnywherePush(
  platform: AnywherePushPlatform,
  onRefresh: () => void,
): () => void {
  if (!platform.supported()) return () => undefined;
  // Notification content is intentionally ignored. Receipt/open is only a hint to refresh the
  // authenticated account state; routing remains inside the app after that refresh.
  return platform.observeRefresh(onRefresh);
}
