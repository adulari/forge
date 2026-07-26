// The neighbouring tab, drawn beside the selected one so a swipe has something to reveal.
//
// Loaded with `React.lazy` rather than imported directly to break a cycle that would otherwise be
// unavoidable: each tab route imports TabPager, so importing the routes back at module scope would
// leave one of the two half-initialised. A dynamic import defers resolution to first use, by which
// point both modules are complete.
//
// NOT preloaded. An earlier version resolved all four modules when the pager mounted, to spend the
// promise's frame before a drag rather than during one, and the app stopped opening —
// "undefined is not a function" — because forcing four route modules (settings.tsx alone pulls some
// 25 local modules) to evaluate inside the first mount reorders initialisation for the whole app.
//
// Each route exports its body as a NAMED export while its default is the TabPager wrapper, so these
// resolve to the body. Pointing them at the default would nest a pager inside a pager and give the
// peek its own peek.
//
// The mapping is written out one branch per tab instead of indexing an array because every JSX
// reference then resolves to a module-level component. That is what `react-hooks/static-components`
// asks for, and the rule is right to ask: a component reference produced during render cannot be
// shown to be stable, and an unstable one silently remounts and drops its state every frame.
import React from "react";
import { StyleSheet, View } from "react-native";

import { PeekProvider } from "../lib/peek";
import { useTokens } from "../theme/ThemeProvider";

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

  // Opaque rather than transparent: nothing behind a peek is meant to be seen, and a see-through gap
  // mid-drag showed as a light flash of the native view underneath.
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
