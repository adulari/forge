const TOKEN_RE = /^[0-9a-f]{16,64}$/i;

export interface ParsedConnectUrl {
  baseUrl: string;
  token: string;
  host: string;
}

/** Parse a Forge connect URL into a canonical token-scoped server origin. */
export function parseConnectUrl(input: string): ParsedConnectUrl | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  const normalized = trimmed.replace(/^connect:/i, "https:");
  let url: URL;
  try {
    url = new URL(normalized);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  const segments = url.pathname.split("/").filter(Boolean);
  const token = segments.at(-1);
  if (!token || !TOKEN_RE.test(token)) return null;
  const baseUrl = `${url.protocol}//${url.host}/${segments.join("/")}`.replace(/\/$/, "");
  return { baseUrl, token, host: url.host };
}
