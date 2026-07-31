import { useLocalSearchParams } from "expo-router";
import React from "react";
import { StyleSheet } from "react-native";

import { Screen } from "../../../components/ds/Screen";
import { WorkspaceDock } from "../../../components/workspace/WorkspaceDock";

export default function SessionFiles() {
  const { id } = useLocalSearchParams<{ id: string }>();

  return (
    <Screen
      edges={["left", "right", "bottom"]}
      scroll={false}
      keyboardAvoiding
      contentContainerStyle={styles.content}
    >
      <WorkspaceDock sessionId={id ?? null} resourceId={null} />
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: 0 },
});
