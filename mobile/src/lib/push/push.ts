// Native no-op push module (ARCHITECTURE.md §2 platform escape hatches: Web
// Push is browser-only; native APNs is a flagged backend gap, FEATURES.md §3).
// Same export shape as `push.web.ts` so callers never branch on platform beyond
// the `isWeb && !isTauri` gate that decides whether to show the Settings row
// at all.
export type PushSubscriptionState = "unsupported" | "subscribed" | "unsubscribed";

export function isPushSupported(): boolean {
  return false;
}

export async function initPush(): Promise<void> {
  // no service worker to register natively
}

export async function getPushStatus(): Promise<PushSubscriptionState> {
  return "unsupported";
}

// Signatures mirror push.web.ts even though the params are unused here — module
// resolution for `./push` is a Metro/bundler-time concern (`.web.ts` on web,
// this file elsewhere); tsc always type-checks against this file's shape, so it
// must match the web module's call signature exactly.
export async function enablePush(_baseUrl: string): Promise<PushSubscriptionState> {
  return "unsupported";
}

export async function disablePush(_baseUrl: string): Promise<PushSubscriptionState> {
  return "unsupported";
}
