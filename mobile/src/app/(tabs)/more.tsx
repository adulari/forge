// More — PLACEHOLDER body. Full search-first launcher (BUILD_PLAN §6 "More") lands in
// Batch 4 (W11).
import { router } from "expo-router";
import React from "react";

import { ListRow, Screen, SectionTitle } from "../../components/ui";

export default function MoreScreen() {
  return (
    <Screen scroll={false}>
      <SectionTitle>More</SectionTitle>
      <ListRow title="Settings" onPress={() => router.push("/settings")} />
    </Screen>
  );
}
