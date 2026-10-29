// Web-only environment checks for the installed-PWA notification prompt.
export function isIOSWeb(): boolean {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent || "";
  if (/iPhone|iPad|iPod/.test(ua)) return true;
  // iPadOS reports itself as a Mac with touch.
  return /Macintosh/.test(ua) && (navigator.maxTouchPoints ?? 0) > 1;
}

export function isStandalonePwa(): boolean {
  if (typeof window === "undefined") return false;
  const nav = navigator as Navigator & { standalone?: boolean };
  return nav.standalone === true || window.matchMedia?.("(display-mode: standalone)").matches === true;
}
