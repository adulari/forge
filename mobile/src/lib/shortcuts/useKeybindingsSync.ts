import { useEffect, useRef } from "react";

import { getKeybindings, putKeybindings } from "../api";
import { useAuth } from "../auth";
import { adoptRemoteShortcuts, parseShortcutOverrides, useShortcutPreferences } from "./bindings";

/**
 * Keeps the app's shortcut overrides in step with the connected daemon's keybindings.json: the
 * daemon's file wins when it has any (so a second device or a hand edit is picked up), otherwise
 * the local overrides seed it. Failures are silent; shortcuts keep working from local storage.
 */
export function useKeybindingsSync(): void {
  const { baseUrl } = useAuth();
  const { loaded, overrides } = useShortcutPreferences();
  const synced = useRef<string | null>(null);
  const known = useRef("");
  const serialized = JSON.stringify(overrides);

  useEffect(() => {
    if (!loaded || !baseUrl || synced.current === baseUrl) return;
    let cancelled = false;
    void getKeybindings(baseUrl)
      .then(async (file) => {
        if (cancelled) return;
        const remote = JSON.stringify(file.app ?? {});
        synced.current = baseUrl;
        if (remote !== "{}") {
          known.current = remote;
          if (remote !== serialized) await adoptRemoteShortcuts(parseShortcutOverrides(remote));
        } else if (serialized !== "{}") {
          known.current = serialized;
          await putKeybindings(baseUrl, overrides);
        } else {
          known.current = "{}";
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [loaded, baseUrl, serialized, overrides]);

  useEffect(() => {
    if (!loaded || !baseUrl || synced.current !== baseUrl || known.current === serialized) return;
    known.current = serialized;
    void putKeybindings(baseUrl, overrides).catch(() => undefined);
  }, [loaded, baseUrl, serialized, overrides]);
}
