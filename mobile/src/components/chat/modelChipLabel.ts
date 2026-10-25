import { shortModelLabel } from "../fleet/shortModelLabel";

/** The composer's model chip. An Automatic session reports the model the mesh last picked; shown
 * bare, that read as a pin the user never set. With the host's pin flag it reads "auto · k3-256k";
 * an older host sends no flag, so the model is shown as before.
 * On a phone a pin shows without its provider prefix, which otherwise cut the name itself off. */
export function modelChipLabel(model: string | null | undefined, pinned: boolean | undefined, compact = false): string {
  if (!model || model === "—") return "auto";
  if (pinned === false) return `auto · ${shortModelLabel(model)}`;
  return compact ? shortModelLabel(model) : model;
}
