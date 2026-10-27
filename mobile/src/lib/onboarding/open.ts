// Opening a sign-in or key page in the system browser. The desktop shell has the opener plugin;
// anywhere else (web preview, tests) fall back to React Native's Linking.
import { Linking } from "react-native";

import { isTauri } from "../platform";

export async function openExternal(url: string): Promise<void> {
  if (!/^https?:\/\//i.test(url)) throw new Error("Only web links can be opened from setup.");
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
    return;
  }
  await Linking.openURL(url);
}
