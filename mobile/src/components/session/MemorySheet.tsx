// Hearth Memory launcher — the sheet is a command launcher, not a data view: durable
// project memories live server-side and open in the conversation/overlay. This sheet
// writes one (`/remember`) and browses the list (`/memories`), and teaches the four
// type badges (handoff pattern 8) the browsed list is grouped by. No memory rows are
// rendered here — the wire carries none to the sheet; results arrive in the transcript.
import { BookOpen, Search } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, type ColorTokens } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

// Handoff pattern 8 type badges: USER ember · PROJECT info · FEEDBACK warn · REFERENCE
// neutral. The DS Badge has no `info` tone, so this renders the same 4-radius small pill
// token-only (bg + ink drawn from ColorTokens, never raw hex) to keep all four distinct.
const MEMORY_TYPES: { label: string; bg: keyof ColorTokens; ink: keyof ColorTokens; meaning: string }[] = [
  { label: "USER", bg: "selection", ink: "accent", meaning: "how you like Forge to work" },
  { label: "PROJECT", bg: "bg3", ink: "info", meaning: "durable facts about this repo" },
  { label: "FEEDBACK", bg: "warnBg", ink: "warn", meaning: "corrections from sessions" },
  { label: "REFERENCE", bg: "bg3", ink: "ink2", meaning: "pointers to docs & checklists" },
];

export function MemorySheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const [memory, setMemory] = useState("");
  const [failed, setFailed] = useState(false);

  const dispatch = (text: string) => {
    if (send({ kind: "prompt", text })) {
      setFailed(false);
      return true;
    }
    setFailed(true);
    return false;
  };
  const remember = () => {
    const value = memory.trim();
    if (!value) return;
    if (dispatch(`/remember ${value}`)) {
      setMemory("");
      onClose();
    }
  };
  const browse = () => {
    if (dispatch("/memories")) onClose();
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Project memory" snapPoints={[0.75]}>
      <View style={[styles.content, isCompact ? null : styles.contentWide]}>
        <Text style={[type.headingBold, { color: tokens.ink }]}>Memory</Text>
        <Text style={[type.sub, styles.subtitle, { color: tokens.ink3 }]}>Durable facts that seed every future session with the right context.</Text>

        <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>remember a fact</Text>
        <Input
          value={memory}
          onChangeText={setMemory}
          placeholder="Prefers exponential backoff with jitter for retries"
          multiline
          accessibilityLabel="New project memory"
          returnKeyType="send"
          onSubmitEditing={remember}
        />
        <Button label="Remember" onPress={remember} disabled={!memory.trim()} fullWidth style={styles.rememberBtn} />

        <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>browse</Text>
        <Button label="Browse project memories" variant="secondary" onPress={browse} fullWidth icon={<Search size={16} strokeWidth={1.75} color={tokens.ink2} />} />
        <View style={styles.hintRow}>
          <BookOpen size={13} strokeWidth={1.75} color={tokens.ink4} />
          <Text style={[type.meta, styles.hint, { color: tokens.ink4 }]}>Opens the full list in the conversation, grouped by type and recall count.</Text>
        </View>

        <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>memory types</Text>
        <View style={[styles.legend, isCompact ? null : styles.legendWide]}>
          {MEMORY_TYPES.map((item) => (
            <View key={item.label} style={[styles.legendRow, isCompact ? null : styles.legendRowWide]}>
              <View style={[styles.badge, { backgroundColor: tokens[item.bg] as string }]}>
                <Text style={[type.meta, styles.badgeText, { color: tokens[item.ink] as string }]}>{item.label}</Text>
              </View>
              <Text style={[type.meta, styles.legendText, { color: tokens.ink3 }]} numberOfLines={1}>
                {item.meaning}
              </Text>
            </View>
          ))}
        </View>

        {failed ? (
          <Text style={[type.sub, styles.error, { color: tokens.danger }]}>Not connected — reconnect to send commands.</Text>
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space32 },
  contentWide: { paddingHorizontal: space.space32 },
  subtitle: { marginTop: 2 },
  section: { paddingTop: space.space20, paddingBottom: space.space8 },
  rememberBtn: { marginTop: space.space12 },
  hintRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space8 },
  hint: { flex: 1 },
  legend: { gap: space.space4 },
  legendWide: { flexDirection: "row", flexWrap: "wrap" },
  legendRow: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 30 },
  legendRowWide: { width: "50%", paddingRight: space.space12 },
  badge: { borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: 2 },
  badgeText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.3 },
  legendText: { flex: 1 },
  error: { marginTop: space.space16, borderRadius: radii.radius8 },
});
