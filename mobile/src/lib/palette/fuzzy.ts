// Palette matching, in the spirit of the TUI's palette.rs (crates/forge-tui/src/commands/palette.rs):
// a prefix hit beats a substring hit beats a subsequence hit. On top of that, the palette also
// matches word initials ("tgr" finds "Toggle Git review"), which is what a typed abbreviation is.
// Scores are higher-is-better; null means no match.

const PREFIX = 900;
const WORD_START = 800;
const INITIALS = 700;
const SUBSTRING = 600;
const SUBSEQUENCE = 200;

function wordStarts(text: string): number[] {
  const starts: number[] = [];
  for (let i = 0; i < text.length; i += 1) {
    const prev = i === 0 ? "" : text[i - 1];
    if (/[a-z0-9]/.test(text[i]) && (i === 0 || /[^a-z0-9]/.test(prev))) starts.push(i);
  }
  return starts;
}

function tokenScore(query: string, text: string, allowSubsequence: boolean): number | null {
  if (text === query) return 1000;
  if (text.startsWith(query)) return PREFIX - Math.min(80, (text.length - query.length) * 0.5);

  const starts = wordStarts(text);
  for (let w = 0; w < starts.length; w += 1) {
    if (text.startsWith(query, starts[w])) return WORD_START - Math.min(60, w * 6);
  }

  const initials = starts.map((i) => text[i]).join("");
  if (query.length >= 2 && initials.startsWith(query)) return INITIALS - Math.min(40, (initials.length - query.length) * 4);

  const at = text.indexOf(query);
  if (at >= 0) return SUBSTRING - Math.min(200, at * 2);

  if (!allowSubsequence) return null;

  let from = 0;
  let previous = -2;
  let score = SUBSEQUENCE;
  const boundaries = new Set(starts);
  for (const char of query) {
    const found = text.indexOf(char, from);
    if (found < 0) return null;
    if (found === previous + 1) score += 6;
    if (boundaries.has(found)) score += 8;
    score -= Math.min(10, found - from);
    previous = found;
    from = found + 1;
  }
  return Math.max(1, Math.min(SUBSTRING - 1, score));
}

/**
 * Every whitespace-separated token must match; the score is their mean. Scattered-letter
 * (subsequence) matches are only allowed where `allowSubsequence` is set: a title is short and
 * typed at, but a subtitle or keyword blob would match almost any three letters.
 */
export function fuzzyScore(query: string, text: string, allowSubsequence = true): number | null {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return 0;
  const haystack = text.toLowerCase();
  let total = 0;
  for (const token of tokens) {
    const score = tokenScore(token, haystack, allowSubsequence);
    if (score == null) return null;
    total += score;
  }
  return total / tokens.length;
}

export interface Rankable {
  title: string;
  subtitle?: string;
  keywords?: string;
  /** Surfaces ahead of equally-scored rows (a session that needs the person stands in for "pinned"). */
  priority?: boolean;
  /** Larger is more recent. */
  recency?: number;
  /** A row the daemon already matched (a message hit): it keeps this score instead of being re-matched. */
  fixedScore?: number;
}

const SUBTITLE_WEIGHT = 0.6;
const KEYWORD_WEIGHT = 0.5;

export function scoreItem(query: string, item: Rankable): number | null {
  if (!query.trim()) return 0;
  if (item.fixedScore != null) {
    const title = fuzzyScore(query, item.title);
    return title != null ? Math.max(title, item.fixedScore) : item.fixedScore;
  }
  const candidates: number[] = [];
  const title = fuzzyScore(query, item.title);
  if (title != null) candidates.push(title);
  if (item.subtitle) {
    const subtitle = fuzzyScore(query, item.subtitle, false);
    if (subtitle != null) candidates.push(subtitle * SUBTITLE_WEIGHT);
  }
  if (item.keywords) {
    const keywords = fuzzyScore(query, item.keywords, false);
    if (keywords != null) candidates.push(keywords * KEYWORD_WEIGHT);
  }
  return candidates.length ? Math.max(...candidates) : null;
}

/** Filter and order: fuzzy score, then priority, then recency, then the order they came in. */
export function rankItems<T extends Rankable>(items: readonly T[], query: string): T[] {
  const scored: { item: T; score: number; index: number }[] = [];
  items.forEach((item, index) => {
    const score = scoreItem(query, item);
    if (score != null) scored.push({ item, score, index });
  });
  scored.sort(
    (a, b) =>
      b.score - a.score ||
      Number(b.item.priority ?? false) - Number(a.item.priority ?? false) ||
      (b.item.recency ?? 0) - (a.item.recency ?? 0) ||
      a.index - b.index,
  );
  return scored.map((entry) => entry.item);
}
