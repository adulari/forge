// Machined desktop shell — "Schedules" (D Schedules, docs/design/machined
// INVENTORY.md L395-403). No recurring/cron-style session scheduling exists yet
// (INVENTORY's gaps summary flags this as a real feature gap) — this is an honest
// empty state, not fabricated schedule rows. Reached from the command palette's
// Actions group.
import { Clock } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Sheet } from "../ds/Sheet";

export interface SchedulesPanelProps {
  visible: boolean;
  onClose: () => void;
}

export function SchedulesPanel({ visible, onClose }: SchedulesPanelProps) {
  const tokens = useTokens();
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Schedules">
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Schedules</Text>
        <EmptyState
          icon={Clock}
          message="Recurring sessions aren't available yet — once scheduling lands, runs would land in Fleet like any other session."
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space8 },
});
