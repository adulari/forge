/** Whether consumers may safely issue requests through a persisted managed host target. */
export function anywhereConsumersReady<T>(
  phase: string,
  runtime: T | null,
  registeredRuntime: T | null,
): boolean {
  if (phase === "loading") return false;
  return runtime === null || runtime === registeredRuntime;
}
