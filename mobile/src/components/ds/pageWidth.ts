import { createContext, useContext } from "react";

/** Every Settings page shares one reading column; wider panes read as sprawl on desktop. */
export const PAGE_MAX_WIDTH = 760;
/** Master/detail pages (Skills, Hooks, Session tree) that lay out two columns at desktop width. */
export const PAGE_WIDE_MAX_WIDTH = 1040;

/** True inside `SettingsShell`: `Screen` then centers its content in the shared column. */
export const PageWidthContext = createContext(false);

export function usePageWidthCap(): boolean {
  return useContext(PageWidthContext);
}
