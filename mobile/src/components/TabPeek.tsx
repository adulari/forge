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

/** Renders nothing for an out-of-range index, so the pager's edges need no special case. */
export function TabPeek({ at }: { at: number }) {
  // `fallback={null}` rather than a skeleton: the first drag is the only one that waits on the
  // import, and an empty gap for a frame reads as depth, where a flash of skeleton reads as a bug.
  return (
    <React.Suspense fallback={null}>
      {at === 0 ? <FleetBody /> : null}
      {at === 1 ? <InboxBody /> : null}
      {at === 2 ? <HistoryBody /> : null}
      {at === 3 ? <SettingsBody /> : null}
    </React.Suspense>
  );
}
