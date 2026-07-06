// Alerts — PLACEHOLDER body. Full implementation (waiting-session filter) lands in
// Batch 1 (W3).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../components/ui";

export default function AlertsScreen() {
  return (
    <Screen>
      <SectionTitle>Alerts</SectionTitle>
      <EmptyState title="Nothing needs you." />
    </Screen>
  );
}
