// Pure half of the notices feature: the model-sent `push_notification` inbox the daemon keeps
// (`GET /api/notices?after=<seq>`, also the Anywhere bridge route `list_notices`). Hosted pushes
// carry only a generic alert, so this is where the real title and message are read — inside the
// authenticated (and, over Anywhere, end-to-end encrypted) connection.
//
// Kept free of react-native imports so the cursor/dedupe/freshness rules are unit-testable.

export interface Notice {
  seq: number;
  /** Unix seconds. */
  at: number;
  title: string;
  message: string;
  priority: string;
  session_id: string;
}

export interface NoticesResponse {
  notices: Notice[];
  latest: number;
}

/** A position in a host's notice log. `seq` restarts at 1 when the daemon restarts, so `at` is
 * compared too: a notice is past a mark if either is greater. */
export interface Mark {
  seq: number;
  at: number;
}

export const ZERO_MARK: Mark = { seq: 0, at: 0 };

/** Only these persist (never the text): where we last alerted, and where the user last looked. */
export interface PersistedMarks {
  notified: Mark;
  seen: Mark;
}

export interface HostNotices extends PersistedMarks {
  /** Newest first, deduped, capped at `MAX_ITEMS`. */
  items: Notice[];
  /** Highest daemon sequence number seen; the `after` of the next fetch. */
  cursor: number;
}

export const MAX_ITEMS = 50;
/** A notice older than this when first fetched is listed but not announced: the daemon inbox
 * outlives the moment it mattered, and a stale batch must not replay as a burst of banners. */
export const FRESH_WINDOW_SECS = 15 * 60;
const MAX_TEXT = 4000;

export function emptyHost(marks?: Partial<PersistedMarks>): HostNotices {
  return {
    items: [],
    cursor: 0,
    notified: marks?.notified ?? ZERO_MARK,
    seen: marks?.seen ?? ZERO_MARK,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseNotice(raw: unknown): Notice | null {
  if (!isRecord(raw)) return null;
  const { seq, at, title, message, priority, session_id } = raw;
  if (typeof seq !== "number" || !Number.isFinite(seq) || seq < 1) return null;
  if (typeof at !== "number" || !Number.isFinite(at)) return null;
  if (typeof message !== "string") return null;
  return {
    seq,
    at,
    title: typeof title === "string" && title.trim() ? title.slice(0, MAX_TEXT) : "Forge",
    message: message.slice(0, MAX_TEXT),
    priority: typeof priority === "string" ? priority : "normal",
    session_id: typeof session_id === "string" ? session_id : "",
  };
}

/** Tolerant of a newer daemon adding fields; rejects anything that is not the documented shape. */
export function parseNoticesResponse(raw: unknown): NoticesResponse {
  if (!isRecord(raw) || !Array.isArray(raw.notices)) {
    throw new Error("unexpected notices response");
  }
  const notices = raw.notices.flatMap((entry) => {
    const parsed = parseNotice(entry);
    return parsed ? [parsed] : [];
  });
  const latest = typeof raw.latest === "number" && Number.isFinite(raw.latest) ? raw.latest : 0;
  return { notices, latest: Math.max(latest, ...notices.map((n) => n.seq)) };
}

export function isAfter(notice: Notice, mark: Mark): boolean {
  return notice.seq > mark.seq || notice.at > mark.at;
}

function markOf(notice: Notice): Mark {
  return { seq: notice.seq, at: notice.at };
}

function laterMark(a: Mark, b: Mark): Mark {
  if (a.at !== b.at) return a.at > b.at ? a : b;
  return a.seq >= b.seq ? a : b;
}

function newestFirst(a: Notice, b: Notice): number {
  return b.at - a.at || b.seq - a.seq;
}

export function noticeKey(notice: Notice): string {
  return `${notice.at}:${notice.seq}`;
}

export interface MergeResult {
  host: HostNotices;
  /** Announce-worthy: past the notified mark and recent. Newest first. */
  fresh: Notice[];
}

export function mergeNotices(host: HostNotices, response: NoticesResponse, nowSecs: number): MergeResult {
  const known = new Set(host.items.map(noticeKey));
  const incoming = response.notices.filter((n) => !known.has(noticeKey(n)));
  const items = [...host.items, ...incoming].sort(newestFirst).slice(0, MAX_ITEMS);
  const fresh = incoming
    .filter((n) => isAfter(n, host.notified) && nowSecs - n.at <= FRESH_WINDOW_SECS)
    .sort(newestFirst);
  const notified = incoming.reduce((mark, n) => laterMark(mark, markOf(n)), host.notified);
  return {
    host: { ...host, items, notified, cursor: Math.max(host.cursor, response.latest) },
    fresh,
  };
}

/**
 * One cursor-based sync. If the daemon reports a `latest` below our cursor it restarted (the log
 * is in memory), so the old items and cursor are void and the log is re-read from the start.
 */
export async function syncHost(
  host: HostNotices,
  fetchAfter: (after: number) => Promise<NoticesResponse>,
  nowSecs: number,
): Promise<MergeResult> {
  let current = host;
  let response = await fetchAfter(current.cursor);
  if (response.latest < current.cursor) {
    current = { ...current, items: [], cursor: 0 };
    response = await fetchAfter(0);
  }
  return mergeNotices(current, response, nowSecs);
}

export function unseenCount(host: HostNotices): number {
  return host.items.filter((n) => isAfter(n, host.seen)).length;
}

export function isUnseen(host: HostNotices, notice: Notice): boolean {
  return isAfter(notice, host.seen);
}

/** Mark everything up to `through` (default: everything held) as seen. Never moves backwards. */
export function markSeen(host: HostNotices, through?: Notice): HostNotices {
  const target = through
    ? markOf(through)
    : host.items.reduce((mark, n) => laterMark(mark, markOf(n)), ZERO_MARK);
  return { ...host, seen: laterMark(host.seen, target) };
}

const SESSION_ID = /^[A-Za-z0-9_-]{1,128}$/;

/** The model's `session_id` is a real session only when it is well-formed and not the daemon's
 * fallback for notices sent from outside any session. */
export function sessionIdOf(notice: Notice): string | null {
  return notice.session_id !== "api" && SESSION_ID.test(notice.session_id) ? notice.session_id : null;
}

export interface BannerCopy {
  title: string;
  body: string;
}

export function bannerCopy(fresh: readonly Notice[]): BannerCopy {
  const [newest] = fresh;
  if (fresh.length <= 1) return { title: newest.title, body: newest.message };
  return { title: `${fresh.length} new notices`, body: `${newest.title}: ${newest.message}` };
}

export const GENERIC_PUSH_BODY = "Open Forge to view an update.";

/** What an OS surface may say. The generic copy mirrors the hosted push alert, so enabling the
 * app lock never leaves a notice's text on the lock screen. */
export function osCopy(notice: Notice, appLocked: boolean): BannerCopy {
  if (appLocked) return { title: "Forge", body: GENERIC_PUSH_BODY };
  return { title: notice.title, body: notice.message };
}

export interface PresentedAlert {
  id: string;
  body: string | null;
  /** Raised by this app (a local notice), not drawn by iOS from a hosted push. */
  own: boolean;
}

/** Hosted generic alerts to clear. Only once `replaced` real-text OS notifications were actually
 * raised: an alert cleared without a replacement is an update the user never gets to see. Our own
 * lock-screen copy shares the generic body and is never touched. */
export function genericAlertsToDismiss(presented: readonly PresentedAlert[], replaced: number): string[] {
  if (replaced <= 0) return [];
  return presented.filter((a) => !a.own && a.body === GENERIC_PUSH_BODY).map((a) => a.id);
}

export interface PersistedState {
  [serverId: string]: PersistedMarks;
}

function parseMark(raw: unknown): Mark {
  if (!isRecord(raw) || typeof raw.seq !== "number" || typeof raw.at !== "number") return ZERO_MARK;
  return { seq: raw.seq, at: raw.at };
}

export function parsePersisted(raw: string | null): PersistedState {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return {};
    const out: PersistedState = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (isRecord(value)) out[id] = { notified: parseMark(value.notified), seen: parseMark(value.seen) };
    }
    return out;
  } catch {
    return {};
  }
}

export type NoticeListRow =
  | { kind: "header"; key: string; label: string }
  | { kind: "notice"; key: string; serverId: string; notice: Notice; unseen: boolean };

/** Flat list rows: one header per host that has notices, newest first within each. Hosts keep the
 * paired-server order so the list does not reshuffle as notices arrive. */
export function buildNoticeRows(
  servers: readonly { id: string; name: string }[],
  hosts: Readonly<Record<string, HostNotices>>,
): NoticeListRow[] {
  const rows: NoticeListRow[] = [];
  for (const server of servers) {
    const host = hosts[server.id];
    if (!host || host.items.length === 0) continue;
    rows.push({ kind: "header", key: `h:${server.id}`, label: server.name });
    for (const notice of host.items) {
      rows.push({
        kind: "notice",
        key: `${server.id}:${noticeKey(notice)}`,
        serverId: server.id,
        notice,
        unseen: isUnseen(host, notice),
      });
    }
  }
  return rows;
}

/** The session's title when the app knows it, else a short id, else null (sent outside a session). */
export function sessionLabel(notice: Notice, titles: ReadonlyMap<string, string>): string | null {
  const id = sessionIdOf(notice);
  if (!id) return null;
  return titles.get(id) || `session ${id.slice(0, 8)}`;
}
