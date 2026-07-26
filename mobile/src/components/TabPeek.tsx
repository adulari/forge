// The neighbouring tab, drawn beside the selected one so a swipe has something to reveal.
//
// Loaded with `React.lazy` rather than imported directly to break a cycle that would otherwise be
// unavoidable: each tab route imports TabPager, so importing the routes back at module scope would
// leave one of the two half-initialised. A dynamic import defers resolution to first use, by which
// point both modules are complete.
//
// Each route exports its body as a NAMED export while its default is the TabPager wrapper, so these
// resolve to the body. Pointing them at the default would nest a pager inside a pager and give the
// peek its own peek.
//
// The mapping is written out one branch per tab instead of indexing an array because every JSX
// reference then resolves to a module-level component. That is what `react-hooks/static-components`
// is asking for, and the rule is right to ask: a component reference produced during render cannot
// be shown to be stable, and an unstable one silently remounts and drops its state every frame.
import React from "react";
import { StyleSheet, View } from "react-native";

import { PeekProvider } from "../lib/peek";
import { useTokens } from "../theme/ThemeProvider";

// Metro bundles every module up front, so these resolve without any network work — but resolution
// is still a promise, which costs a frame. That frame is visible at the very start of a drag as a
// flash of nothing where the neighbour should be, so `preloadTabPeeks` settles them in advance and
// the loaders are kept separate from `React.lazy` to make that possible.
const loaders = [
  () => import("../app/(tabs)/index"),
  () => import("../app/(tabs)/inbox"),
  () => import("../app/(tabs)/history"),
  () => import("../app/(tabs)/settings"),
] as const;

const FleetBody = React.lazy(() => loaders[0]().then((m) => ({ default: m.FleetScreen })));
const InboxBody = React.lazy(() => loaders[1]().then((m) => ({ default: m.InboxScreen })));
const HistoryBody = React.lazy(() => loaders[2]().then((m) => ({ default: m.HistoryScreen })));
const SettingsBody = React.lazy(() => loaders[3]().then((m) => ({ default: m.SettingsScreen })));

/**
 * Resolves every neighbour module ahead of the first drag. Idempotent and fire-and-forget: a failed
 * import is not worth reporting, because `React.lazy` will surface it if the peek is ever rendered.
 */
export function preloadTabPeeks(): void {
  for (const load of loaders) void load().catch(() => undefined);
}

export function TabPeek({ at }: { at: number }) {
  const tokens = useTokens();

  // An opaque fallback rather than `null`: nothing behind a peek is meant to be seen, and a
  // transparent gap reads as a flicker. With preloading this should never actually render.
  return (
    <View style={[styles.fill, { backgroundColor: tokens.bg0 }]}>
      <PeekProvider>
        <React.Suspense fallback={null}>
          {at === 0 ? <FleetBody /> : null}
          {at === 1 ? <InboxBody /> : null}
          {at === 2 ? <HistoryBody /> : null}
          {at === 3 ? <SettingsBody /> : null}
        </React.Suspense>
      </PeekProvider>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
});
