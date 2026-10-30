import { Platform } from "react-native";

function webLocation(): Location | null {
  if (Platform.OS !== "web") return null;
  if (typeof window === "undefined" || typeof window.location === "undefined") return null;
  return window.location;
}

/** A pairing link the page was opened with (`/#token=..&fp=..`), or null. */
export function readPairingFromLocation(): string | null {
  const here = webLocation();
  if (!here || !/(^|[#&])token=/.test(here.hash)) return null;
  return `${here.origin}/${here.hash}`;
}

/** Drop the token from the address bar and history so it is not bookmarked, synced or shared. */
export function stripPairingFromLocation(): void {
  const here = webLocation();
  if (!here || typeof window.history?.replaceState !== "function") return;
  if (!/(^|[#&])token=/.test(here.hash) && !/[?&]url=/.test(here.search)) return;
  window.history.replaceState(window.history.state, "", here.pathname);
}
