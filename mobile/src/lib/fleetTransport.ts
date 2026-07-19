/** Whether the selected transport exposes the direct-daemon fleet invalidation socket. */
export function supportsFleetInvalidationSocket(baseUrl: string): boolean {
  const protocol = new URL(baseUrl).protocol;
  return protocol !== "fany:" && protocol !== "fany-ws:";
}
