import type { EvidenceImage } from "./mediaApi";

export interface GalleryEntry {
  file: string;
  title: string;
  tool: string;
  at: number;
  page: string | null;
}

const TOOL_LABEL: Record<string, string> = { browser: "Browser", device: "Device" };

/** The gallery's entries, newest first, with a readable caption and tool for each screenshot. */
export function galleryEntries(items: readonly EvidenceImage[]): GalleryEntry[] {
  return items
    .map((item) => ({
      file: item.file,
      title: item.caption?.trim() || pageTitle(item.url) || item.file,
      tool: TOOL_LABEL[item.source] ?? item.source,
      at: Date.parse(item.ts),
      page: item.url,
    }))
    .reverse();
}

function pageTitle(url: string | null): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname === "/" ? "" : parsed.pathname}`;
  } catch {
    return null;
  }
}
