// Chat segment — PLACEHOLDER. Full implementation (history merge + streaming + action
// cards + input bar) lands in Batch 2/3 (BUILD_PLAN §7, W5/W6).
import { useLocalSearchParams } from "expo-router";
import React from "react";

import { EmptyState, Screen, SectionTitle } from "../../../components/ui";

export default function ChatScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return (
    <Screen>
      <SectionTitle>{`Chat — ${id}`}</SectionTitle>
      <EmptyState title="Connecting to session…" />
    </Screen>
  );
}
