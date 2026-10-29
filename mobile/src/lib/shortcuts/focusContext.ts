import type { FocusTag } from "./when";

const published = new Set<string>();

/** The shell publishes tags it owns state for (`hasSession`); DOM focus zones are read on demand. */
export function publishFocusTag(tag: FocusTag, on: boolean): void {
  if (on) published.add(tag);
  else published.delete(tag);
}

const ZONE_TAGS: Record<string, FocusTag> = {
  composer: "composerFocus",
  terminal: "terminalFocus",
  sidebar: "sidebarFocus",
  editor: "editorFocus",
};

export function currentFocusTags(active: Element | null = typeof document === "undefined" ? null : document.activeElement): Set<string> {
  const tags = new Set<string>(published);
  const zone = active?.closest?.("[data-focus-zone]")?.getAttribute("data-focus-zone");
  const tag = zone ? ZONE_TAGS[zone] : undefined;
  if (tag) tags.add(tag);
  return tags;
}
