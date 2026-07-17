// Hearth Checkpoint launcher — save a restorable snapshot, or open the server-owned
// checkpoint picker (`picker:checkpoints`). The picker rows + the restore-confirm
// decision card live in the overlay flow (OverlayHost), NOT here; this sheet is the
// entry point and explains the restore contract in copy. `CheckpointOverlayRows` is
// the Hearth renderer for those picker rows — wired into NativeOverlayContent.
import { Camera, RotateCcw } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { Overlay, RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space, tapTarget } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function CheckpointSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [name, setName] = useState("");
  const save = () => {
    if (send({ kind: "prompt", text: name.trim() ? `/checkpoint ${name.trim()}` : "/checkpoint" })) {
      setName("");
      onClose();
    }
  };
  const restore = () => {
    if (send({ kind: "prompt", text: "/checkpoints" })) onClose();
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Session checkpoints" snapPoints={[0.62]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Checkpoint</Text>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          A checkpoint captures every file state and this conversation at this turn — a safe point to return to before a risky change.
        </Text>

        <Text style={[typeScale.meta, styles.fieldLabel, { color: tokens.ink3 }]}>Name (optional)</Text>
        <Input
          value={name}
          onChangeText={setName}
          placeholder="before comparator rewrite"
          autoCapitalize="sentences"
          numberOfLines={1}
          accessibilityLabel="Checkpoint name"
          returnKeyType="send"
          onSubmitEditing={save}
        />

        <Button label="Save checkpoint" onPress={save} fullWidth icon={<Camera size={16} strokeWidth={2} color={tokens.bg2} />} />
        <Button label="Browse & restore" variant="secondary" onPress={restore} fullWidth icon={<RotateCcw size={16} strokeWidth={2} color={tokens.accent} />} />

        <Text style={[typeScale.meta, styles.restoreNote, { color: tokens.ink4 }]}>
          Restoring rewinds files and the conversation to a checkpoint. A checkpoint of “now” is taken first, so nothing is lost — you can jump back anytime.
        </Text>
      </View>
    </Sheet>
  );
}

// ---------------------------------------------------------------------------
// picker:checkpoints — Hearth checkpoint rows for the server overlay flow.
// OverlayRow carries { label, detail, selected, group }; we style the checkpoint
// dot from the label kind ("manual" = ember, "auto" = quiet) and render the
// cp-id / turn / diff-stat detail verbatim in mono (server-formatted, not parsed).
// ---------------------------------------------------------------------------

export const CHECKPOINT_OVERLAY_KIND = "picker:checkpoints";

function dotColor(label: string, selected: boolean, tokens: ReturnType<typeof useTokens>): string {
  if (selected) return tokens.accent;
  return /^\s*manual/i.test(label) ? tokens.accent : tokens.ink4;
}

export function CheckpointOverlayRows({ overlay, onSelect }: { overlay: Overlay; onSelect: (id: string) => void }) {
  const tokens = useTokens();
  return (
    <ScrollView style={styles.rows} keyboardShouldPersistTaps="handled">
      {overlay.rows.map((row, index) => (
        <Pressable
          key={row.id}
          onPress={() => onSelect(row.id)}
          accessibilityRole="menuitem"
          accessibilityState={{ selected: row.selected }}
          accessibilityLabel={row.detail ? `${row.label} — ${row.detail}` : row.label}
          style={[
            styles.overlayRow,
            index > 0 ? { borderTopColor: tokens.hairline, borderTopWidth: StyleSheet.hairlineWidth } : null,
            row.selected ? { backgroundColor: tokens.selection } : null,
          ]}
        >
          <View style={[styles.dot, { backgroundColor: dotColor(row.label, row.selected, tokens) }]} />
          <View style={styles.overlayCopy}>
            <Text style={[typeScale.bodyBold, { color: row.selected ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{row.label}</Text>
            {row.detail ? <Text style={[styles.mono, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>{row.detail}</Text> : null}
          </View>
        </Pressable>
      ))}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24, gap: space.space8 },
  subtitle: { marginTop: 2, marginBottom: space.space4 },
  fieldLabel: { marginTop: space.space4 },
  restoreNote: { marginTop: space.space8, lineHeight: 16 },
  rows: { flex: 1 },
  overlayRow: { minHeight: tapTarget, flexDirection: "row", alignItems: "center", gap: space.space12, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  overlayCopy: { flex: 1, gap: space.space2, minWidth: 0 },
  dot: { width: 7, height: 7, borderRadius: 3.5 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
});
