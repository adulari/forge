/** Whether consumers may safely issue requests through a persisted managed host target. */
export function anywhereConsumersReady<T>(
  phase: string,
  runtime: T | null,
  registeredRuntime: T | null,
): boolean {
  if (phase === "loading") return false;
  if (phase !== "ready") return true;
  return runtime === null || runtime === registeredRuntime;
}

export function phaseAfterSetupRestart(
  hasCredentials: boolean,
  reauthenticationRequired: boolean,
): "reauthentication_required" | "ready" | "signed_out" {
  if (hasCredentials && reauthenticationRequired) return "reauthentication_required";
  return hasCredentials ? "ready" : "signed_out";
}
