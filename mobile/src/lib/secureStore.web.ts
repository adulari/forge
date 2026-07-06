// Web shim for secureStore.ts — expo-secure-store is a no-op on web, so Expo web
// (the primary fast-QA target per BUILD_PLAN §9a) falls back to localStorage.
// Not a security boundary on web; the daemon token still lives behind HTTPS + the
// path-segment auth model regardless of storage medium.

export async function getSecureItem(key: string): Promise<string | null> {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(key);
}

export async function setSecureItem(key: string, value: string): Promise<void> {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(key, value);
}

export async function deleteSecureItem(key: string): Promise<void> {
  if (typeof localStorage === "undefined") return;
  localStorage.removeItem(key);
}
