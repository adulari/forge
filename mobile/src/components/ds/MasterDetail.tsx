// DESIGN_SYSTEM.md §7 Responsive layout — MasterDetail: compact/medium stack
// (route navigation handles the "stack"), expanded (>=1024) renders a
// persistent left rail (320pt) + right detail pane. Route files stay
// identical; expo-router renders the same screens into either layout.
import React from "react";
import { StyleSheet, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { useBreakpoint } from "../../theme/useBreakpoint";

export interface MasterDetailProps {
  /** The list/rail content (e.g. fleet list + inbox pills + New Session). `null` collapses the
   * rail (Hearth: settings-family routes bring their own 240px nav rail) while keeping the
   * detail pane mounted in a stable tree position, so route/screen state survives the toggle. */
  master: React.ReactNode | null;
  /** The session/content detail. Only rendered at the expanded breakpoint. */
  detail?: React.ReactNode;
  /** Rail width at the expanded breakpoint. Default 316 (Hearth desktop/web shells). */
  railWidth?: number;
}

export function MasterDetail({ master, detail, railWidth = 316 }: MasterDetailProps) {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();

  if (!isExpanded) {
    return <View style={styles.flex}>{master}</View>;
  }

  return (
    <View style={styles.row}>
      <View
        style={
          master == null
            ? styles.railCollapsed
            : [styles.rail, { width: railWidth, borderRightColor: tokens.border, backgroundColor: tokens.bg1 }]
        }
      >
        {master}
      </View>
      <View style={[styles.flex, { backgroundColor: tokens.bg1 }]}>{detail}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  row: { flex: 1, flexDirection: "row" },
  rail: { borderRightWidth: StyleSheet.hairlineWidth },
  railCollapsed: { width: 0, overflow: "hidden" },
});
