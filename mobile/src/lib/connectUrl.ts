const TOKEN_RE = /^[0-9a-f]{16,64}$/i;

export interface ParsedConnectUrl {
  baseUrl: string;
  token: string;
  host: string;
  /** `sha256:<hex>` of the daemon's TLS leaf certificate, when the link carries a pin. */
  fingerprint?: string;
}

export function normalizeFingerprint(raw: string): string | null {
  const text = raw.trim().toLowerCase().replace(/^sha-?256[:=]/, "").replace(/:/g, "");
  return /^[0-9a-f]{64}$/.test(text) ? `sha256:${text}` : null;
}

/**
 * Parse a Forge connect URL into a canonical token-scoped server origin. Two shapes parse:
 * the pairing link `https://host:port/#token=<hex>&fp=sha256:<hex>` and the older
 * `https://host:port/<token>` path form (which carries no pin).
 */
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

  const fragment = new URLSearchParams(url.hash.replace(/^#/, ""));
  const fragmentToken = fragment.get("token");
  if (fragmentToken !== null) {
    if (!TOKEN_RE.test(fragmentToken)) return null;
    const rawPin = fragment.get("fp");
    const fingerprint = rawPin === null ? undefined : (normalizeFingerprint(rawPin) ?? undefined);
    if (rawPin !== null && !fingerprint) return null;
    const prefix = segments.length > 0 ? `/${segments.join("/")}` : "";
    const baseUrl = `${url.protocol}//${url.host}${prefix}/${fragmentToken}`;
    return { baseUrl, token: fragmentToken, host: url.host, ...(fingerprint ? { fingerprint } : {}) };
  }

  const token = segments.at(-1);
  if (!token || !TOKEN_RE.test(token)) return null;
  const baseUrl = `${url.protocol}//${url.host}/${segments.join("/")}`.replace(/\/$/, "");
  return { baseUrl, token, host: url.host };
}

export interface ConnectTarget {
  /** Host as it will be dialled, lowercased, never the token. */
  host: string;
  cleartext: boolean;
  pinned: boolean;
  /** Set when the host mixes scripts or uses punycode, which is how lookalike hosts hide. */
  lookalike: boolean;
  private: boolean;
}

function isPrivateHost(hostname: string): boolean {
  if (hostname === "localhost" || hostname.endsWith(".local") || hostname.endsWith(".lan")) return true;
  if (hostname.endsWith(".ts.net")) return true;
  const m = /^(\d+)\.(\d+)\.\d+\.\d+$/.exec(hostname);
  if (!m) return false;
  const [a, b] = [Number(m[1]), Number(m[2])];
  return a === 10 || a === 127 || (a === 192 && b === 168) || (a === 172 && b >= 16 && b <= 31) || (a === 100 && b >= 64 && b <= 127);
}

/** What the confirm card shows. Built from the parsed URL, never from the raw input. */
export function describeConnectTarget(parsed: ParsedConnectUrl): ConnectTarget {
  const url = new URL(parsed.baseUrl);
  const hostname = url.hostname.toLowerCase();
  return {
    host: url.host.toLowerCase(),
    cleartext: url.protocol === "http:",
    pinned: parsed.fingerprint !== undefined,
    lookalike: hostname.split(".").some((label) => label.startsWith("xn--")) || /[^\x00-\x7f]/.test(hostname),
    private: isPrivateHost(hostname.replace(/^\[|\]$/g, "")),
  };
}

export function composeManualLink(host: string, port: string, token: string, fingerprint?: string): string | null {
  const cleanHost = host.trim().replace(/^https?:\/\//i, "").replace(/\/.*$/, "");
  const cleanToken = token.trim();
  if (!cleanHost || !cleanToken) return null;
  const portPart = port.trim() ? `:${port.trim()}` : "";
  const pin = fingerprint ? `&fp=${fingerprint}` : "";
  return `https://${cleanHost}${portPart}/#token=${cleanToken}${pin}`;
}
