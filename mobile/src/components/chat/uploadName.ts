/** The name a user gave an uploaded file. The daemon stores uploads as
 * `<ms timestamp>-<16 hex nonce>-<original name>` (older builds omitted the nonce); both prefixes
 * are storage details, and the nonce used to show up in the chat as "3f1c3ddda03c0938-notes.txt". */
export function uploadDisplayName(path: string): string {
  const base = path.split("/").pop() ?? path;
  return base.replace(/^\d+-(?:[0-9a-f]{16}-)?/, "");
}
