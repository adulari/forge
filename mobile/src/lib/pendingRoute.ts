// The route an unpaired visitor deep-linked to, so pairing can return there instead of /(tabs).
let pending: string | null = null;

export function rememberPendingRoute(path: string): void {
  if (path && path !== "/" && !path.startsWith("/connect")) pending = path;
}

export function takePendingRoute(): string | null {
  const out = pending;
  pending = null;
  return out;
}
