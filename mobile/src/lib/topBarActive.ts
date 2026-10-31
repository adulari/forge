// Every route reached from the Settings rail belongs to the Settings tab; without this the
// top bar showed no active tab on any sub-page.
const SETTINGS_FAMILY = /^\/(settings|appearance|desktop|keybindings|diagnostics|legal|anywhere|usage|providers|models|plans|mcp|configuration|skills|hooks|control|session-tree|workflows|prs|remote-access)(\/|$)/;

/** `/session-tree` is a Settings page, not a session, so Fleet matches `/session` only as a whole segment. */
export function isTopBarTabActive(href: string, pathname: string): boolean {
  if (href === "/") return pathname === "/" || /^\/session(\/|$)/.test(pathname);
  if (href === "/settings") return SETTINGS_FAMILY.test(pathname);
  return pathname.startsWith(href);
}
