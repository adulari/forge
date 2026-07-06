// New session (modal) — PLACEHOLDER body. Full form (cwd/title/model/worktree toggle +
// useCreateSession mutation) lands in Batch 1 (BUILD_PLAN §7, W2).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../components/ui";

export default function NewSessionScreen() {
  return (
    <Screen>
      <SectionTitle>New session</SectionTitle>
      <EmptyState title="Session creation form coming soon." />
    </Screen>
  );
}
