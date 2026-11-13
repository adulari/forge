// Merge queue as a settings page: the PRs Forge is landing one at a time, per repo, with the
// owner's controls. Reached from Pull requests and the command palette.
import React from "react";

import { PageHeader } from "../components/ds/PageHeader";
import { Screen } from "../components/ds/Screen";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { MergeQueueView } from "../components/pr/MergeQueueView";
import { SettingsShell } from "./(tabs)/settings";
import { space } from "../theme/tokens";

export default function MergeQueueScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="merge-queue">
        <Screen scroll contentContainerStyle={{ paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }}>
          <PageHeader
            title="Merge queue"
            subtitle="Pull requests Forge lands one at a time. Hold one to pause it, or move one to the front."
          />
          <MergeQueueView />
        </Screen>
      </SettingsShell>
    </DesktopDrillDown>
  );
}
