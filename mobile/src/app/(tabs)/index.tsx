// Fleet — PLACEHOLDER body. Full implementation (BUILD_PLAN §6 "Fleet") lands in Batch 1
// (W2). This nav-shell version only proves the FAB → /new-session route and the tab body.
import { router } from "expo-router";
import React from "react";
import { View } from "react-native";

import { EmptyState, FAB, Screen, SectionTitle } from "../../components/ui";

export default function FleetScreen() {
  return (
    <Screen>
      <SectionTitle>Fleet</SectionTitle>
      <EmptyState title="No live sessions — start one" />
      <View className="h-16" />
      <FAB label="New" onPress={() => router.push("/new-session")} />
    </Screen>
  );
}
