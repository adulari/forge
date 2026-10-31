import React from "react";
import { StyleSheet, View } from "react-native";

import { Sheet } from "../ds/Sheet";
import { PortsDock } from "./PortsDock";

/** The Ports and actions view for phones, where the workbench docks do not exist. */
export function PortsSheet({ visible, onClose, sessionId }: { visible: boolean; onClose: () => void; sessionId: string }) {
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Ports and project actions" snapPoints={[0.75]}>
      <View style={styles.body}>
        <PortsDock sessionId={sessionId} onNavigate={onClose} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({ body: { flex: 1 } });
