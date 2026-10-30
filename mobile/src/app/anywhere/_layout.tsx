import { Stack, usePathname } from "expo-router";
import React from "react";

import { useAuth } from "../../lib/auth";
import { SettingsShell } from "../(tabs)/settings";

/** Anywhere is a Settings section: every page under it keeps the Settings rail, so
 * drilling into Devices or Storage no longer drops the user out of Settings. Before
 * pairing (Connect deep-links here) there is no Settings to return to. */
export default function AnywhereLayout() {
  const { isPaired } = useAuth();
  const pathname = usePathname();
  const stack = <Stack screenOptions={{ headerShown: false }} />;
  if (!isPaired) return stack;
  return <SettingsShell active={/^\/anywhere\/sync\/?$/.test(pathname) ? "anywhere-sync" : "anywhere"}>{stack}</SettingsShell>;
}
