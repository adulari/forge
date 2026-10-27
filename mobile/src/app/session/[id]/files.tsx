import { useSessionStable } from "../../../lib/sessionContext";
import React from "react";
import { StyleSheet } from "react-native";

import { Screen } from "../../../components/ds/Screen";
import { WorkspaceDock } from "../../../components/workspace/WorkspaceDock";
import { useSegmentsHosted } from "../../../components/session/SegmentKeepAlive";

export function SessionFiles() {
  const { sessionId: id } = useSessionStable();

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

// Desktop/web: the session layout keeps this segment mounted itself (SegmentKeepAlive), so the
// route renders nothing there; phones render the screen as the route.
export default function SessionFilesRoute() {
  return useSegmentsHosted() ? null : <SessionFiles />;
}
