/** `provider::vendor/model` → `model`; the daemon's "—" sentinel (Automatic, nothing picked yet)
 * → `auto`. The full id is one tap away in the session header. */
export function shortModelLabel(model: string): string {
  if (!model || model === "—") return "auto";
  const afterProvider = model.includes("::") ? model.slice(model.lastIndexOf("::") + 2) : model;
  return afterProvider.slice(afterProvider.lastIndexOf("/") + 1) || model;
}
