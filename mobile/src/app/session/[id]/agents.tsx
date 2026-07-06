// Agents segment — PLACEHOLDER. Full implementation (snapshot.subagents cards) lands in
// Batch 3 (BUILD_PLAN §7, W7).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../../components/ui";

export default function AgentsScreen() {
  return (
    <Screen>
      <SectionTitle>Agents</SectionTitle>
      <EmptyState title="No subagents running." />
    </Screen>
  );
}
