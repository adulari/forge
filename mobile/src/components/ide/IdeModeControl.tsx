// The Chat | IDE switch in the title bar. Two words, no icons: it should read at a glance and
// stay out of the way of the layout control beside it.
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii } from "../../theme/tokens";
import { sansFamilyFor } from "../../theme/typography";
import { useShortcutLabel } from "../../lib/shortcuts";
import { rowBackground } from "./rowStyle";
import { useIdeMode, type ShellMode } from "./useIdeMode";

const OPTIONS: readonly { value: ShellMode; label: string }[] = [
  { value: "chat", label: "Chat" },
  { value: "ide", label: "IDE" },
];

export function IdeModeControl() {
  const tokens = useTokens();
  const { available, mode, setMode } = useIdeMode();
  const hint = useShortcutLabel("ide.toggle");
  if (!available) return null;
  return (
    <View style={[styles.track, { borderColor: tokens.border }]} accessibilityRole="tablist" accessibilityLabel={`Mode. Switch with ${hint}`}>
      {OPTIONS.map((option) => {
        const selected = option.value === mode;
        return (
          <Pressable
            key={option.value}
            onPress={() => setMode(option.value)}
            accessibilityRole="tab"
            accessibilityState={{ selected }}
            accessibilityHint={option.value === "ide" ? `IDE mode, ${hint}` : `Chat, ${hint}`}
            style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [
              styles.option,
              { backgroundColor: selected ? rowBackground(tokens, { selected: true }) : rowBackground(tokens, { hovered }) },
            ]}
          >
            <Text style={[styles.label, { color: selected ? tokens.ink : tokens.ink3 }]}>{option.label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  track: { flexDirection: "row", borderWidth: 1, borderRadius: radii.radius4, overflow: "hidden" },
  option: { height: 22, minWidth: 38, paddingHorizontal: 9, alignItems: "center", justifyContent: "center" },
  label: { fontFamily: sansFamilyFor(500), fontSize: 11 },
});
