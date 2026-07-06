// Overlay mirror (modal) — PLACEHOLDER. Full implementation (auto present/dismiss on
// snapshot.overlay, palette/pickers/config/usage/mesh/workflow) lands in Batch 4
// (BUILD_PLAN §7, W9).
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../../components/ui";

export default function OverlayScreen() {
  return (
    <Screen>
      <SectionTitle>Overlay</SectionTitle>
      <EmptyState title="No overlay active." />
    </Screen>
  );
}
