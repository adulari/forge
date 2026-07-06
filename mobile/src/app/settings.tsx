// Settings — PLACEHOLDER body. Full server-info/test-connection/re-pair/forget flow
// lands in Batch 1 (BUILD_PLAN §7, W1).
import React from "react";

import { ListRow, Screen, SectionTitle } from "../components/ui";
import { useAuth } from "../lib/auth";

export default function SettingsScreen() {
  const { host } = useAuth();
  return (
    <Screen scroll={false}>
      <SectionTitle>Settings</SectionTitle>
      <ListRow title="Server" subtitle={host ?? "not paired"} />
    </Screen>
  );
}
