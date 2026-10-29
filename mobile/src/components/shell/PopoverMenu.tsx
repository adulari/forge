// A small anchored menu for the desktop shell (title-bar Layout menu, session context menu).
// Same look as the server menu in DesktopWindowChrome: bg2 panel, hairline border, 30px rows,
// mono hint on the right, an accent check for on/selected items. Rendered in a transparent Modal
// so it floats above every pane and dock and closes on outside click or Escape.
import { Check } from "lucide-react-native";
import React from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export type MenuEntry =
  | {
      kind: "item";
      id: string;
      label: string;
      hint?: string;
      checked?: boolean;
      disabled?: boolean;
      onPress: () => void;
    }
  | { kind: "heading"; id: string; label: string }
  | { kind: "separator"; id: string };

export interface MenuAnchor {
  top: number;
  /** Distance from the window's right edge — the menu grows leftwards. */
  right?: number;
  left?: number;
}

export interface PopoverMenuProps {
  anchor: MenuAnchor | null;
  entries: readonly MenuEntry[];
  onClose: () => void;
  accessibilityLabel: string;
  width?: number;
}

export function PopoverMenu({ anchor, entries, onClose, accessibilityLabel, width = 280 }: PopoverMenuProps) {
  const tokens = useTokens();
  const { height: windowHeight } = useWindowDimensions();
  return (
    <Modal visible={anchor != null} transparent animationType="none" onRequestClose={onClose}>
      <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityLabel={`Close ${accessibilityLabel}`} />
      {anchor ? (
        <View
          accessibilityRole="menu"
          accessibilityLabel={accessibilityLabel}
          style={[
            styles.menu,
            {
              top: anchor.top,
              right: anchor.right,
              left: anchor.left,
              width,
              maxHeight: Math.max(160, windowHeight - anchor.top - 12),
              backgroundColor: tokens.bg2,
              borderColor: tokens.border,
            },
          ]}
        >
          <ScrollView>
            {entries.map((entry) => {
              if (entry.kind === "separator") {
                return <View key={entry.id} style={[styles.divider, { backgroundColor: tokens.border }]} />;
              }
              if (entry.kind === "heading") {
                return (
                  <Text key={entry.id} style={[typeScale.section, styles.heading, { color: tokens.ink4 }]}>
                    {entry.label}
                  </Text>
                );
              }
              return (
                <Pressable
                  key={entry.id}
                  disabled={entry.disabled}
                  onPress={() => {
                    onClose();
                    entry.onPress();
                  }}
                  accessibilityRole="menuitem"
                  accessibilityLabel={entry.label}
                  accessibilityHint={entry.hint}
                  accessibilityState={{ checked: entry.checked, disabled: entry.disabled }}
                  style={({ pressed, hovered, focused }: { pressed: boolean; hovered?: boolean; focused?: boolean }) => [
                    styles.item,
                    (hovered || focused || pressed) && !entry.disabled && { backgroundColor: tokens.bg3 },
                  ]}
                >
                  <View style={styles.check}>{entry.checked ? <Check size={12} color={tokens.accent} strokeWidth={2} /> : null}</View>
                  <Text style={[styles.label, entry.disabled && styles.disabled, { color: entry.checked ? tokens.accent : tokens.ink }]} numberOfLines={1}>
                    {entry.label}
                  </Text>
                  {/* A refused action says why, so its reason stays readable while the label dims. */}
                  {entry.hint ? <Text style={[styles.hint, { color: entry.disabled ? tokens.ink3 : tokens.ink4 }]}>{entry.hint}</Text> : null}
                </Pressable>
              );
            })}
          </ScrollView>
        </View>
      ) : null}
    </Modal>
  );
}

const styles = StyleSheet.create({
  menu: {
    position: "absolute",
    borderWidth: 1,
    borderRadius: radii.radius8,
    paddingVertical: space.space4,
    shadowColor: "#000",
    shadowOpacity: 0.35,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 6 },
  },
  item: { flexDirection: "row", alignItems: "center", gap: space.space8, height: 30, paddingHorizontal: space.space12 },
  disabled: { opacity: 0.4 },
  check: { width: 12 },
  label: { flex: 1, fontSize: 12.5 },
  hint: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  heading: { paddingHorizontal: space.space12, paddingTop: space.space8, paddingBottom: space.space4, fontFamily: monoFamily.regular },
  divider: { height: StyleSheet.hairlineWidth, marginVertical: space.space4 },
});
