// The slim bar under the workbench: the terminal toggle on the left, the review keys on the right.
// Only keys that do something right now are listed.
import { Terminal as TerminalIcon } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { useShortcutLabel } from "../../lib/shortcuts";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { sansFamilyFor } from "../../theme/typography";
import { useIde } from "./IdeContext";
import { Keycap } from "./ideUi";
import { rowBackground } from "./rowStyle";

export const KEY_HINTS_HEIGHT = 28;

function Hint({ keys, label }: { keys: string[]; label: string }) {
  const tokens = useTokens();
  return (
    <View style={styles.hint}>
      <View style={styles.keys}>
        {keys.map((key) => (
          <Keycap key={key} label={key} />
        ))}
      </View>
      <Text style={[styles.label, { color: tokens.ink3 }]}>{label}</Text>
    </View>
  );
}

export function KeyHints() {
  const tokens = useTokens();
  const { width } = useWindowDimensions();
  const { active, rows, bottomOpen, toggleBottom } = useIde();
  const terminal = useShortcutLabel("workbench.terminal");
  const reviewing = active?.kind === "diff";
  const wide = width >= 1180;
  return (
    <View style={[styles.root, { backgroundColor: tokens.bg1, borderTopColor: tokens.border }]}>
      <Pressable
        onPress={toggleBottom}
        accessibilityRole="button"
        accessibilityState={{ expanded: bottomOpen }}
        accessibilityLabel="Toggle terminal"
        style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.terminal, { backgroundColor: rowBackground(tokens, { hovered, selected: bottomOpen }) }]}
      >
        <TerminalIcon size={12} strokeWidth={1.75} color={tokens.ink2} />
        <Text style={[styles.label, { color: tokens.ink2 }]}>Terminal</Text>
        <Keycap label={terminal} />
      </Pressable>
      <View style={styles.spacer} />
      {reviewing ? (
        <>
          <Hint keys={["j", "k"]} label="files" />
          {wide ? <Hint keys={["n", "p"]} label="changes" /> : null}
          <Hint keys={["r"]} label="reviewed" />
          <Hint keys={["m"]} label="message" />
        </>
      ) : rows.length > 0 ? (
        <Hint keys={["j"]} label="open the first file" />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { height: KEY_HINTS_HEIGHT, flexDirection: "row", alignItems: "center", gap: space.space16, paddingHorizontal: space.space8, borderTopWidth: StyleSheet.hairlineWidth },
  terminal: { height: 22, flexDirection: "row", alignItems: "center", gap: 6, paddingHorizontal: space.space8, borderRadius: 4 },
  spacer: { flex: 1 },
  hint: { flexDirection: "row", alignItems: "center", gap: 6 },
  keys: { flexDirection: "row", gap: 2 },
  label: { fontFamily: sansFamilyFor(500), fontSize: 11 },
});
