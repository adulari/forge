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

// NOT preloaded, and that is deliberate. An earlier version resolved all four of these when the
// pager mounted, to spend the promise's frame before a drag rather than during one. It shipped as
// #903 and the app failed to open: "undefined is not a function" from the root error boundary.
//
// The eager preload was the only thing that release ran at app OPEN — the peek-aware refetch and the
// dropped haptic can only take effect during a drag — and forcing four route modules (settings.tsx
// alone pulls some 25 local modules) to evaluate inside the first mount reorders initialisation for
// the whole app. Resolving them on first use instead keeps evaluation where the code was written to
// expect it: after the tree that those modules depend on already exists.
//
// The flash it was meant to fix is handled by the opaque fallback below instead, which costs nothing
// and cannot reorder anything.
const FleetBody = React.lazy(() =>
  import("../app/(tabs)/index").then((m) => ({ default: m.FleetScreen })),
);
const InboxBody = React.lazy(() =>
  import("../app/(tabs)/inbox").then((m) => ({ default: m.InboxScreen })),
);
const HistoryBody = React.lazy(() =>
  import("../app/(tabs)/history").then((m) => ({ default: m.HistoryScreen })),
);
const SettingsBody = React.lazy(() =>
  import("../app/(tabs)/settings").then((m) => ({ default: m.SettingsScreen })),
);

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
