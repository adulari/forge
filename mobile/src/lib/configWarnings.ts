import type { ConfigResponse } from "./api";

/** The daemon's warning for one settings field, or null: shown under the field so a half-set
 * `[harness.expert]` says what is missing instead of looking saved and working. */
export function configWarning(data: ConfigResponse | undefined, key: string): string | null {
  return data?.warnings?.find((warning) => warning.key === key)?.text ?? null;
}
