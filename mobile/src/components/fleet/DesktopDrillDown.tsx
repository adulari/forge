// Hearth desktop/web drill-down passthrough. `ExpandedFleetRail` (the 316px Fleet
// rail this file used to also export) is retired — Machined wave 2 replaces it with
// `components/shell/Sidebar.tsx` + `IconRail.tsx`, rendered directly from
// `app/_layout.tsx`'s `RootNavigator`. This file now only keeps the passthrough
// wrapper every settings-family screen (usage.tsx, models.tsx, hooks.tsx, etc.)
// still imports — those call sites are unrelated to the rail and out of scope here.
import React from "react";

export function DesktopDrillDown({ children }: { children: React.ReactNode }) {
  return children;
}
