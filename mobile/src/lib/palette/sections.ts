// Pure palette structure: the category chips, capped sections with "N more" rows, and the
// keyboard index maths. The component only renders what these return.

export type PaletteCategory = "all" | "sessions" | "commands" | "files" | "settings";
export type PaletteSection = Exclude<PaletteCategory, "all">;

export const PALETTE_CATEGORIES: readonly { key: PaletteCategory; label: string }[] = [
  { key: "all", label: "All" },
  { key: "sessions", label: "Sessions" },
  { key: "commands", label: "Commands" },
  { key: "files", label: "Files" },
  { key: "settings", label: "Settings" },
];

export const SECTION_ORDER: readonly PaletteSection[] = ["sessions", "commands", "files", "settings"];

export const SECTION_LABELS: Record<PaletteSection, string> = {
  sessions: "Sessions",
  commands: "Commands",
  files: "Files",
  settings: "Settings",
};

/** Rows one section shows in the All view before collapsing into "N more". */
export const ALL_VIEW_CAP = 5;
/** Rows a single-category view shows before "N more" expands the rest. */
export const CATEGORY_VIEW_CAP = 40;
export const PAGE_STEP = 8;

export function cycleCategory(current: PaletteCategory, direction: 1 | -1): PaletteCategory {
  const index = PALETTE_CATEGORIES.findIndex((c) => c.key === current);
  const next = (index + direction + PALETTE_CATEGORIES.length) % PALETTE_CATEGORIES.length;
  return PALETTE_CATEGORIES[next].key;
}

export interface SectionedItem {
  id: string;
  section: PaletteSection;
}

export interface MoreRow {
  kind: "more";
  id: string;
  section: PaletteSection;
  hidden: number;
}

export interface ItemRow<T> {
  kind: "item";
  id: string;
  item: T;
}

export type PaletteRow<T> = ItemRow<T> | MoreRow;

export interface SectionBlock<T> {
  section: PaletteSection;
  rows: PaletteRow<T>[];
}

export function moreRowId(section: PaletteSection): string {
  return `more:${section}`;
}

/**
 * Group already-ranked items into sections. The All view shows `ALL_VIEW_CAP` per section and a
 * "N more" row that drills into that section; a single category view shows up to
 * `CATEGORY_VIEW_CAP` and the "N more" row there expands the rest in place.
 */
export function buildSections<T extends SectionedItem>(
  ranked: readonly T[],
  category: PaletteCategory,
  expanded: ReadonlySet<PaletteSection>,
): SectionBlock<T>[] {
  const blocks: SectionBlock<T>[] = [];
  for (const section of SECTION_ORDER) {
    if (category !== "all" && category !== section) continue;
    const items = ranked.filter((item) => item.section === section);
    if (items.length === 0) continue;
    const cap = category === "all" ? ALL_VIEW_CAP : expanded.has(section) ? Number.POSITIVE_INFINITY : CATEGORY_VIEW_CAP;
    const rows: PaletteRow<T>[] = items.slice(0, cap).map((item) => ({ kind: "item", id: item.id, item }));
    if (items.length > cap) {
      rows.push({ kind: "more", id: moreRowId(section), section, hidden: items.length - cap });
    }
    blocks.push({ section, rows });
  }
  return blocks;
}

/** Flat keyboard order: rows that can take the selection (disabled items cannot). */
export function selectableRows<T extends { disabled?: boolean }>(blocks: readonly SectionBlock<T>[]): PaletteRow<T>[] {
  return blocks.flatMap((block) => block.rows).filter((row) => row.kind === "more" || !row.item.disabled);
}

/** Arrow keys wrap around the ends. */
export function stepIndex(current: number, delta: 1 | -1, length: number): number {
  if (length <= 0) return 0;
  return (current + delta + length) % length;
}

/** PageUp/PageDown move by `PAGE_STEP` and stop at the ends. */
export function pageIndex(current: number, direction: 1 | -1, length: number): number {
  if (length <= 0) return 0;
  return Math.max(0, Math.min(length - 1, current + direction * PAGE_STEP));
}

/** Enter during an IME composition confirms the candidate, never the palette row. */
export function isImeComposing(event: { isComposing?: boolean; keyCode?: number }): boolean {
  return event.isComposing === true || event.keyCode === 229;
}

export function filesHeader(project: string | null, branch: string | null): string {
  if (!project) return "File names";
  return branch ? `File names in ${project} (${branch})` : `File names in ${project}`;
}

export type FilesEmptyState = "no-session" | "short-query" | "idle";

export function filesEmptyState(hasSession: boolean, query: string): FilesEmptyState {
  if (!hasSession) return "no-session";
  return query.trim().length < 2 ? "short-query" : "idle";
}

export const FILES_EMPTY_COPY: Record<Exclude<FilesEmptyState, "idle">, string> = {
  "no-session": "Select a project to search files",
  "short-query": "Type at least two characters",
};
