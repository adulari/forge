/** The routes IDE mode draws itself: the fleet home and a session. Everything else (settings,
 * inbox, history, new session) is a full page the chat shell owns, so IDE mode steps aside for it. */
export function isIdeRoute(pathname: string): boolean {
  return pathname === "/" || pathname === "" || /^\/session\/[^/]+/.test(pathname);
}
