// Pure half of automatic issue reporting: scrub, classify, fingerprint, build. No storage, no
// network, no React Native imports, so every privacy rule is unit-testable. The wire contract is
// docs/features/issue-reporting.md (relay POST /v1/reports, schema v1).

export type ReportingMode = "auto" | "ask" | "off";
export const REPORTING_MODES: readonly ReportingMode[] = ["auto", "ask", "off"];

export type ErrorKind =
  | "js_error"
  | "js_unhandled_rejection"
  | "react_boundary"
  | "tauri_command"
  | "tauri_panic"
  | "manual";

export type ReportSurface = "desktop" | "ios" | "android" | "web";

export interface ReportEvent {
  k: string;
  ms?: number;
  ago: number;
}

export interface ReportContext {
  version: string;
  git_sha: string;
  surface: ReportSurface;
  os: string;
  arch: string;
  os_version?: string;
  install_method?: string;
  flags: string[];
}

export interface Report {
  v: 1;
  source: "auto" | "manual";
  install_id: string;
  fingerprint: string;
  error_kind: ErrorKind;
  site: string;
  location: string;
  message: string;
  frames: string[];
  context: ReportContext;
  events: ReportEvent[];
  user_note?: string;
  last_error?: string;
  occurred_at: number;
}

export const MAX_MESSAGE = 300;
export const MAX_NOTE = 2000;
export const MAX_LAST_ERROR = 1000;
export const MAX_FRAMES = 12;
export const MAX_EVENTS = 40;
export const MANUAL_FINGERPRINT = "0000000000000000";

const REDACTED = "[redacted]";

const SECRET_SHAPES: RegExp[] = [
  /\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi,
  /\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}/g,
  /\bsk-[A-Za-z0-9_-]{12,}/g,
  /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{16,}/g,
  /\bgithub_pat_[A-Za-z0-9_]{20,}/g,
  /\bxox[abprs]-[A-Za-z0-9-]{10,}/g,
  /\bAKIA[0-9A-Z]{16}\b/g,
  /\bAIza[0-9A-Za-z_-]{30,}/g,
  /\b(?:api[_-]?key|apikey|token|secret|password|passwd|authorization)(["']?\s*[:=]\s*["']?)[^\s"',;}&]{4,}/gi,
];

function scrubSecrets(text: string): string {
  let out = text;
  for (const shape of SECRET_SHAPES) {
    out = out.replace(shape, (match, ...groups) => {
      const lead = typeof groups[0] === "string" ? groups[0] : null;
      if (lead !== null && /^["']?\s*[:=]/.test(lead)) {
        const key = match.slice(0, match.indexOf(lead));
        return `${key}${lead}${REDACTED}`;
      }
      return REDACTED;
    });
  }
  // Long opaque runs (hex, base64, ids) that contain a digit: credential-shaped, never useful.
  return out.replace(/[A-Za-z0-9+/_-]{32,}={0,2}/g, (run) => (/\d/.test(run) ? REDACTED : run));
}

function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}

function hostOf(url: string): string {
  const match = /^[a-z][a-z0-9+.-]*:\/\/(?:[^/@\s]*@)?(\[[^\]]+\]|[^/:?#\s]+)/i.exec(url);
  return match ? match[1] : "";
}

/**
 * Remove everything user-identifying from free text: secrets, emails, URLs (kept as host only),
 * IP addresses, local hostnames, home directories (`~`) and other absolute paths (basename).
 * Truncates to `max` characters.
 */
export function scrubText(input: string, max = MAX_MESSAGE): string {
  let out = String(input ?? "");
  out = out.replace(/\b[a-z][a-z0-9+.-]*:\/\/[^\s"'<>)\]}]+/gi, (url) => {
    const host = hostOf(url);
    return host ? `<url:${host}>` : "<url>";
  });
  out = scrubSecrets(out);
  out = out.replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, "[email]");
  out = out.replace(/\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b/g, "[ip]");
  out = out.replace(/\b(?:[0-9a-f]{1,4}:){3,7}[0-9a-f]{1,4}\b/gi, "[ip]");
  out = out.replace(
    /\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:local|lan|internal|home|corp|ts\.net)\b/gi,
    "[host]",
  );
  out = out.replace(/(?:[A-Za-z]:\\Users\\[^\\\s]+|\/(?:home|Users)\/[^/\s]+)(?=[\\/]|\b)/g, "~");
  out = out.replace(/(^|[\s"'(=:[])((?:[A-Za-z]:)?(?:[\\/][^\\/\s"'()<>:]+){2,})/g, (_m, lead, path) => {
    if (String(path).startsWith("~")) return `${lead}${path}`;
    return `${lead}${basename(path)}`;
  });
  out = out.replace(/(^|[\s"'(=:[])(~(?:[\\/][^\\/\s"'()<>:]+)+)/g, (_m, lead, path) => {
    return `${lead}~/${basename(path)}`;
  });
  out = out.replace(/\s+/g, " ").trim();
  return out.length > max ? `${out.slice(0, max - 1)}…` : out;
}

/**
 * Drop quoted strings, backtick spans and {...} blobs from an error message: the places where a
 * prompt, a reply, tool arguments or file contents hide inside error text. Messages only; text a
 * user typed on purpose keeps its quotes.
 */
export function stripContent(input: string): string {
  let out = String(input ?? "");
  out = out.replace(/"(?:[^"\\]|\\.)*"/g, "<str>");
  out = out.replace(/(^|[\s(\[:=,])'[^'\n]*'/g, "$1<str>");
  out = out.replace(/`[^`]*`/g, "<code>");
  for (let i = 0; i < 4; i += 1) {
    const next = out.replace(/\{[^{}]*\}/g, "{..}");
    if (next === out) break;
    out = next;
  }
  return out;
}

/** Free text a user typed on purpose: same scrubbing, longer limit, newlines preserved. */
export function scrubUserText(input: string, max: number): string {
  return String(input ?? "")
    .split("\n")
    .map((line) => scrubText(line, max))
    .join("\n")
    .trim()
    .slice(0, max);
}

export interface ParsedFrame {
  fn: string;
  file: string;
  line: number | null;
}

const FRAME_LINE = /^\s*at\s+(?:(.+?)\s+\()?(.+?)(?::(\d+))?(?::\d+)?\)?\s*$/;
const FRAME_AT = /^\s*(.*?)@(.+?)(?::(\d+))?(?::\d+)?\s*$/;

export function parseStack(stack: string | undefined): ParsedFrame[] {
  if (!stack) return [];
  const frames: ParsedFrame[] = [];
  for (const raw of stack.split("\n").slice(0, 60)) {
    const m = FRAME_LINE.exec(raw) ?? (raw.includes("@") ? FRAME_AT.exec(raw) : null);
    if (!m) continue;
    const fn = (m[1] ?? "").replace(/^(?:async|new|Object\.|Module\.)\s*/g, "").trim();
    const file = m[2] ?? "";
    const line = m[3] ? Number.parseInt(m[3], 10) : null;
    if (/native code|node_modules|react-native\/|react-dom|scheduler|^\[native\]|^<anonymous>/i.test(file)) {
      continue;
    }
    frames.push({ fn, file, line });
  }
  return frames;
}

function normalizeFrameName(fn: string): string {
  const cleaned = fn
    .replace(/[^A-Za-z0-9_$.<>]/g, "")
    .replace(/\$[0-9a-f]{4,}/gi, "")
    .replace(/^Object\./, "");
  return cleaned.slice(0, 120);
}

export function frameNames(stack: string | undefined): string[] {
  const names: string[] = [];
  for (const frame of parseStack(stack)) {
    const name = normalizeFrameName(frame.fn);
    if (name && name !== "anonymous" && !names.includes(name)) names.push(name);
    if (names.length >= MAX_FRAMES) break;
  }
  return names;
}

export function topLocation(stack: string | undefined): string {
  const [top] = parseStack(stack);
  if (!top) return "";
  // Bundles are content-hashed (entry-3f9a8b7c6d5e.js): drop the hash so it neither reads as a token
  // to the scrubber nor changes the problem identity between releases.
  const file = basename(top.file.split("?")[0]).replace(/[-_.][0-9A-Za-z]{8,}(?=\.m?c?js$)/, "");
  return scrubText(top.line ? `${file}:${top.line}` : file, 160);
}

/** Stable 64-bit-ish hash as 16 hex chars (two independent 32-bit FNV/murmur-style lanes). */
export function stableHash16(input: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x9e3779b9;
  for (let i = 0; i < input.length; i += 1) {
    const c = input.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 0x01000193);
    h2 = Math.imul(h2 ^ c, 0x85ebca6b);
    h2 ^= h2 >>> 13;
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 0x85ebca6b) ^ Math.imul(h2 ^ (h2 >>> 13), 0xc2b2ae35);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 0x27d4eb2f) ^ Math.imul(h1 ^ (h1 >>> 15), 0x165667b1);
  return `${(h1 >>> 0).toString(16).padStart(8, "0")}${(h2 >>> 0).toString(16).padStart(8, "0")}`;
}

/** Error kind + site + top normalized frames. Never the message: it can carry user data. */
export function fingerprintOf(kind: ErrorKind, site: string, frames: string[]): string {
  return stableHash16([kind, site, ...frames.slice(0, 4)].join("|"));
}

export type Classification = { report: true } | { report: false; reason: string };

const EXPECTED_MESSAGE =
  /network request failed|failed to fetch|networkerror|load failed|network error|timed? ?out|timeout|offline|internet connection|econn|enotfound|etimedout|socket hang up|aborted|cancell?ed|unauthori[sz]ed|forbidden|rate.?limit|too many requests|quota|invalid (?:url|config)|not configured|permission denied|user denied|not allowed by the user/i;

/**
 * Whether a JS error is a bug worth reporting. Expected conditions are NOT reported: the user's
 * network or server being down, aborted/cancelled requests, auth failures, rate limits, and
 * user-configuration errors. HTTP errors from the user's own daemon (anything with a numeric
 * `status`) are expected conditions too.
 */
export function classifyError(kind: ErrorKind, error: unknown): Classification {
  if (kind === "manual") return { report: true };
  const err = error as { name?: unknown; message?: unknown; status?: unknown; code?: unknown } | null;
  const name = typeof err?.name === "string" ? err.name : "";
  const message = typeof err?.message === "string" ? err.message : typeof error === "string" ? error : "";
  if (name === "AbortError" || name === "ApiError" || name === "TimeoutError") {
    return { report: false, reason: "expected_" + name.toLowerCase() };
  }
  if (typeof err?.status === "number") return { report: false, reason: "http_status" };
  if (name === "NetworkError" || (name === "TypeError" && /fetch|network|load failed/i.test(message))) {
    return { report: false, reason: "network" };
  }
  if (EXPECTED_MESSAGE.test(message)) return { report: false, reason: "expected_condition" };
  if (message.trim() === "" && !name) return { report: false, reason: "empty" };
  return { report: true };
}

export function slug(value: string, max = 32): string {
  return value.toLowerCase().replace(/[^a-z0-9_.-]/g, "_").slice(0, max);
}

export interface BuildInput {
  source: "auto" | "manual";
  kind: ErrorKind;
  site: string;
  error?: unknown;
  context: ReportContext;
  installId: string;
  events: readonly { k: string; ms?: number; at: number }[];
  now: number;
  userNote?: string;
  lastError?: string;
}

function errorFields(error: unknown): { message: string; stack?: string } {
  if (error instanceof Error) return { message: error.message, stack: error.stack };
  if (error && typeof error === "object") {
    const e = error as { message?: unknown; stack?: unknown };
    return {
      message: typeof e.message === "string" ? e.message : "",
      stack: typeof e.stack === "string" ? e.stack : undefined,
    };
  }
  return { message: typeof error === "string" ? error : "" };
}

export function buildReport(input: BuildInput): Report {
  const { message, stack } = errorFields(input.error);
  const frames = frameNames(stack);
  const site = input.site.replace(/[^A-Za-z0-9_:.<>/-]/g, "_").slice(0, 120);
  const report: Report = {
    v: 1,
    source: input.source,
    install_id: input.installId,
    fingerprint: input.source === "manual" ? MANUAL_FINGERPRINT : fingerprintOf(input.kind, site, frames),
    error_kind: input.kind,
    site,
    location: topLocation(stack),
    message: scrubText(stripContent(message), MAX_MESSAGE),
    frames,
    context: input.context,
    events: input.events.slice(-MAX_EVENTS).map((e) => ({
      k: slug(e.k),
      ...(e.ms !== undefined ? { ms: Math.max(0, Math.round(e.ms)) } : {}),
      ago: Math.max(0, Math.round((input.now - e.at) / 1000)),
    })),
    occurred_at: Math.floor(input.now / 1000),
  };
  if (input.userNote) report.user_note = scrubUserText(input.userNote, MAX_NOTE);
  if (input.lastError) report.last_error = scrubUserText(input.lastError, MAX_LAST_ERROR);
  return report;
}

export const NEVER_SENT = [
  "Your prompts and the model's replies",
  "File contents, code, diffs and tool results",
  "Command lines and environment variables",
  "API keys, tokens and passwords",
  "Email addresses, host names, user names and absolute paths (paths are cut to a file name, URLs to a host)",
];
