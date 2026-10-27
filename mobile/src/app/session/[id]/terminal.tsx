import { useSessionStable } from "../../../lib/sessionContext";
import React from "react";
import { StyleSheet } from "react-native";

import { Screen } from "../../../components/ds/Screen";
import { useSegmentsHosted } from "../../../components/session/SegmentKeepAlive";
import { TerminalDock } from "../../../components/shell/TerminalDock";

export function SessionTerminal() {
  const { sessionId: id } = useSessionStable();

  return (
    <Screen
      edges={["left", "right", "bottom"]}
      scroll={false}
      keyboardAvoiding
      contentContainerStyle={styles.content}
    >
      <TerminalDock sessionId={id ?? null} compact />
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: 0 },
});

export default function SessionTerminalRoute() {
  return useSegmentsHosted() ? null : <SessionTerminal />;
}
