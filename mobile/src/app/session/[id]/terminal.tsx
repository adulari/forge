import { useLocalSearchParams } from "expo-router";
import React from "react";
import { StyleSheet } from "react-native";

import { Screen } from "../../../components/ds/Screen";
import { TerminalDock } from "../../../components/shell/TerminalDock";

export default function SessionTerminal() {
  const { id } = useLocalSearchParams<{ id: string }>();

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
