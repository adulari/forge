// History — PLACEHOLDER body. Full implementation (infinite past sessions + resume
// flow) lands in Batch 1 (W3).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../components/ui";

export default function HistoryScreen() {
  return (
    <Screen>
      <SectionTitle>History</SectionTitle>
      <EmptyState title="No past sessions yet." />
    </Screen>
  );
}
