// Review segment — PLACEHOLDER. Full implementation (plan card + diff card) lands in
// Batch 3 (BUILD_PLAN §7, W8).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../../components/ui";

export default function ReviewScreen() {
  return (
    <Screen>
      <SectionTitle>Review</SectionTitle>
      <EmptyState title="Nothing to review." />
    </Screen>
  );
}
