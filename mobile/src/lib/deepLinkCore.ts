// `forge://…` links a second launch hands to the running desktop app (src-tauri/src/desktop_shell.rs).
// Only a closed set of in-app destinations is accepted; everything else is dropped.
const SESSION_ID = /^[A-Za-z0-9_-]{1,128}$/;

export function routeForDeepLink(link: string): string | null {
  const match = /^forge:\/\/([^?#]*)/.exec(link.trim());
  if (!match) return null;
  const parts = match[1].split("/").filter(Boolean);
  const [head, id, ...rest] = parts;
  if (rest.length > 0) return null;
  switch (head) {
    case "session":
      return id && SESSION_ID.test(id) ? `/session/${id}` : null;
    case "new":
      return id ? null : "/new-session";
    case "inbox":
      return id ? null : "/inbox";
    case "fleet":
      return id ? null : "/";
    default:
      return null;
  }
}
