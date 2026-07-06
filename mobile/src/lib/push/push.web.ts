// Web Push (ARCHITECTURE.md §5 web target, §2 platform escape hatches;
// FEATURES.md §1.1/§5). Subscribe flow: register `public/sw.js` → request
// Notification permission → GET /api/push/key (VAPID public key) →
// `PushManager.subscribe` → POST /api/push/subscribe. Unsubscribe: the
// browser subscription's own `.unsubscribe()` → POST /api/push/unsubscribe
// (best-effort — the local unsubscribe already took effect either way).
//
// Feature-detected throughout: browsers without serviceWorker/PushManager/
// Notification (older Safari, or a webview without these APIs) report
// "unsupported" rather than throwing, so callers never need a try/catch.
import { getPushKey, subscribePush as apiSubscribePush, unsubscribePush as apiUnsubscribePush } from "../api";

export type PushSubscriptionState = "unsupported" | "subscribed" | "unsubscribed";

const SW_URL = "/sw.js";

// sw.js can't reach `localStorage` (no synchronous storage in a service worker
// context) or the app's own baseUrl state, so notificationclick needs its own
// copy of the active daemon's baseUrl to POST /api/answer. IndexedDB is the
// one storage medium both this page and public/sw.js can read — the DB/store/
// key names here MUST match public/sw.js's `getStoredBaseUrl` verbatim.
const DB_NAME = "forge-push";
const STORE_NAME = "kv";
const BASE_URL_KEY = "baseUrl";

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore(STORE_NAME);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function storeBaseUrl(baseUrl: string): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      tx.objectStore(STORE_NAME).put(baseUrl, BASE_URL_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } catch {
    // best-effort — sw.js falls back to opening the session (without
    // answering the prompt) if this write never landed
  }
}

export function isPushSupported(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    "serviceWorker" in navigator &&
    "PushManager" in window &&
    "Notification" in window
  );
}

/** Registers the service worker. Idempotent — re-registering the same script is a browser no-op. */
export async function initPush(): Promise<void> {
  if (!isPushSupported()) return;
  try {
    await navigator.serviceWorker.register(SW_URL);
  } catch {
    // best-effort; enablePush()/getPushStatus() surface real failures to the caller
  }
}

/** VAPID public key (base64url, RFC 8291) → the `Uint8Array` `applicationServerKey` wants. */
function applicationServerKey(base64url: string): Uint8Array {
  const padded = base64url.replace(/-/g, "+").replace(/_/g, "/");
  const pad = (4 - (padded.length % 4)) % 4;
  const raw = atob(padded + "=".repeat(pad));
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

async function getRegistration(): Promise<ServiceWorkerRegistration | null> {
  if (!isPushSupported()) return null;
  try {
    return (await navigator.serviceWorker.getRegistration(SW_URL)) ?? (await navigator.serviceWorker.ready);
  } catch {
    return null;
  }
}

export async function getPushStatus(): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";
  const reg = await getRegistration();
  if (!reg) return "unsubscribed";
  const sub = await reg.pushManager.getSubscription();
  return sub ? "subscribed" : "unsubscribed";
}

export async function enablePush(baseUrl: string): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";

  await initPush();
  const reg = await getRegistration();
  if (!reg) return "unsubscribed";

  const permission = await Notification.requestPermission();
  if (permission !== "granted") return "unsubscribed";

  let sub = await reg.pushManager.getSubscription();
  if (!sub) {
    const { key } = await getPushKey(baseUrl);
    sub = await reg.pushManager.subscribe({
      userVisibleOnly: true,
      // lib.dom's generic `Uint8Array<ArrayBufferLike>` vs. the stricter
      // `BufferSource` (`ArrayBufferView<ArrayBuffer>`) parameter type is a
      // TS-lib variance artifact, not a real mismatch — a freshly allocated
      // `Uint8Array` always satisfies `BufferSource` at runtime.
      applicationServerKey: applicationServerKey(key) as BufferSource,
    });
  }

  const json = sub.toJSON();
  await apiSubscribePush(baseUrl, {
    endpoint: json.endpoint ?? sub.endpoint,
    keys: {
      p256dh: json.keys?.p256dh ?? "",
      auth: json.keys?.auth ?? "",
    },
  });
  await storeBaseUrl(baseUrl);

  return "subscribed";
}

export async function disablePush(baseUrl: string): Promise<PushSubscriptionState> {
  if (!isPushSupported()) return "unsupported";

  const reg = await getRegistration();
  const sub = reg ? await reg.pushManager.getSubscription() : null;
  if (sub) {
    const endpoint = sub.endpoint;
    await sub.unsubscribe();
    try {
      await apiUnsubscribePush(baseUrl, { endpoint });
    } catch {
      // local unsubscribe already succeeded; the daemon will prune a stale
      // endpoint on its next failed send regardless
    }
  }

  return "unsubscribed";
}
