// Tasks segment — PLACEHOLDER. Full implementation (snapshot.tasks list) lands in
// Batch 3 (BUILD_PLAN §7, W7).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../../components/ui";

export default function TasksScreen() {
  return (
    <Screen>
      <SectionTitle>Tasks</SectionTitle>
      <EmptyState title="No task list yet." />
    </Screen>
  );
}
