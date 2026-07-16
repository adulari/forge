const SAFE_SCHEMES = new Set(["http:", "https:", "mailto:"]);

/** Return a user-openable link or null for executable/privileged URL schemes. */
export function safeExternalHref(raw: string): string | null {
  const href = raw.trim();
  if (!href) return null;
  if (href.startsWith("#")) return href;
  try {
    const parsed = new URL(href);
    return SAFE_SCHEMES.has(parsed.protocol) ? href : null;
  } catch {
    return null;
  }
}
