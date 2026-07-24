// Machined desktop shell — the right-side dock container (D Split + Usage / W Main,
// docs/design/machined INVENTORY.md): 280-296px, hairline left border, a typed
// registry of docked panels. Only the Usage dock is implemented (wave 2 scope) — the
// registry shape is what lets a future terminal/git-review/notes dock slot in later
// as one more entry, not a rewrite of this container.
import { X } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { UsageDock } from "./UsageDock";

/** Future docks (terminal, git review, notes) are follow-up PRs — add them here as
 * more union members + `DOCK_REGISTRY` entries, nothing else in this file changes. */
export type DockKind = "usage";

interface DockDefinition {
  title: string;
  render: () => React.ReactNode;
}

const DOCK_REGISTRY: Record<DockKind, DockDefinition> = {
  usage: { title: "Usage", render: () => <UsageDock /> },
};

const DOCK_WIDTH = 288;

export interface DockHostProps {
  open: boolean;
  dock?: DockKind;
  onClose: () => void;
}

export function DockHost({ open, dock = "usage", onClose }: DockHostProps) {
  const tokens = useTokens();
  if (!open) return null;
  const definition = DOCK_REGISTRY[dock];

  return (
    <View style={[styles.dock, { width: DOCK_WIDTH, borderLeftColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        <Text style={[typeScale.bodyBold, styles.title, { color: tokens.ink }]}>{definition.title}</Text>
        <Pressable
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={`Close ${definition.title.toLowerCase()} dock`}
          accessibilityHint="Command U"
          style={styles.closeButton}
        >
          <X size={14} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </View>
      <View style={styles.body}>{definition.render()}</View>
    </View>
  );
}

const styles = StyleSheet.create({
  dock: { flexShrink: 0, borderLeftWidth: StyleSheet.hairlineWidth },
  header: {
    height: 38,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space12,
  },
  title: { flex: 1 },
  closeButton: { width: 26, height: 26, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
});
