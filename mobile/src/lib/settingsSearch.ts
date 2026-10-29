import type { ConfigField } from "./api";
import { rankItems } from "./palette/fuzzy";
import { SETTINGS_ENTRIES, type SettingsHref } from "./palette/settingsEntries";

export const CONFIG_SECTION_ORDER = [
  "Notifications",
  "Terminal",
  "Threads",
  "Git",
  "Worktrees",
  "Audio",
  "Archived",
] as const;

const SECTION_PATTERNS: readonly [section: (typeof CONFIG_SECTION_ORDER)[number], pattern: RegExp][] = [
  ["Notifications", /(^|[._])(notif\w*|push|alerts?)([._]|$)/i],
  ["Worktrees", /(^|[._])worktrees?([._]|$)/i],
  ["Terminal", /(^|[._])(terminals?|shell|pty)([._]|$)/i],
  ["Audio", /(^|[._])(audio|voice|speech|tts|stt|mic\w*)([._]|$)/i],
  ["Archived", /(^|[._])archiv\w*([._]|$)/i],
  ["Git", /(^|[._])(git|coauthor)([._]|$)/i],
  ["Threads", /(^|[._])(threads?|sessions?|chat|follow_up)([._]|$)/i],
];

export function configSection(field: Pick<ConfigField, "key" | "group">): string {
  for (const [section, pattern] of SECTION_PATTERNS) {
    if (pattern.test(field.key)) return section;
  }
  return field.group;
}

export function groupConfigFields(fields: readonly ConfigField[]): [string, ConfigField[]][] {
  const result = new Map<string, ConfigField[]>();
  for (const field of fields) {
    const section = configSection(field);
    const bucket = result.get(section) ?? [];
    bucket.push(field);
    result.set(section, bucket);
  }
  const rank = (name: string) => {
    const index = (CONFIG_SECTION_ORDER as readonly string[]).indexOf(name);
    return index < 0 ? -1 : index;
  };
  return [...result.entries()].sort((a, b) => rank(a[0]) - rank(b[0]));
}

const FIELD_KEYWORDS: Record<string, string> = {
  "power.keep_awake": "sleep awake caffeinate suspend idle power battery",
  "tui.theme": "colour color appearance dark light",
  "tui.reduce_motion": "animation accessibility",
  permission_mode: "approve safety sandbox bypass",
};

export interface SettingsHit {
  id: string;
  kind: "page" | "field";
  title: string;
  subtitle: string;
  keywords: string;
  href: SettingsHref;
  focusKey?: string;
}

export function settingsIndex(fields: readonly ConfigField[]): SettingsHit[] {
  const pages: SettingsHit[] = SETTINGS_ENTRIES.map((entry) => ({
    id: `page:${entry.href}`,
    kind: "page",
    title: entry.title,
    subtitle: entry.subtitle,
    keywords: entry.keywords,
    href: entry.href,
  }));
  const rows: SettingsHit[] = fields.map((field) => ({
    id: `field:${field.key}`,
    kind: "field",
    title: field.label,
    subtitle: `Configuration · ${configSection(field)}`,
    keywords: `${field.key} ${field.help ?? ""} ${FIELD_KEYWORDS[field.key] ?? ""}`,
    href: "/configuration",
    focusKey: field.key,
  }));
  return [...pages, ...rows];
}

export function searchSettings(query: string, fields: readonly ConfigField[], limit = 30): SettingsHit[] {
  if (!query.trim()) return [];
  return rankItems(settingsIndex(fields), query).slice(0, limit);
}

export function configHref(focusKey?: string) {
  return focusKey ? ({ pathname: "/configuration", params: { focus: focusKey } } as const) : ("/configuration" as const);
}
