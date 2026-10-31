// The context a prompt pulls in with `@session:<name>`, `@terminal`, `@browser` and
// `@Subagents`, shown as chips. In the composer each chip takes its mention back out of the
// draft; under a sent message the row is read-only.
import { Bot, Globe, MessageSquare, SquareTerminal } from "lucide-react-native";
import React, { useMemo } from "react";
import { ScrollView, StyleSheet, View } from "react-native";

import { draftMentions, type MentionKind } from "../../lib/mentionsModel";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { Chip } from "../ds/Chip";

export function MentionIcon({ kind, color, size = 14 }: { kind: MentionKind; color: string; size?: number }) {
  const props = { size, strokeWidth: 1.75, color };
  switch (kind) {
    case "session":
      return <MessageSquare {...props} />;
    case "terminal":
      return <SquareTerminal {...props} />;
    case "browser":
      return <Globe {...props} />;
    default:
      return <Bot {...props} />;
  }
}

interface MentionChipsProps {
  text: string;
  /** Takes one mention out of the draft. Without it the chips are display only. */
  onRemove?: (token: string) => void;
}

export function MentionChips({ text, onRemove }: MentionChipsProps) {
  const tokens = useTokens();
  const mentions = useMemo(() => draftMentions(text), [text]);
  if (mentions.length === 0) return null;
  const chips = mentions.map((mention) => (
    <Chip
      key={mention.token}
      size="compact"
      selected
      label={mention.kind === "session" ? `session ${mention.label}` : mention.label}
      icon={<MentionIcon kind={mention.kind} color={tokens.accent} />}
      onPress={onRemove ? () => onRemove(mention.token) : undefined}
      testID={`mention-chip-${mention.kind}`}
    />
  ));
  if (!onRemove) return <View style={styles.readOnly}>{chips}</View>;
  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.row}
      keyboardShouldPersistTaps="handled"
      accessibilityLabel="Context pulled in by mentions. Press a chip to remove it."
    >
      {chips}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  row: { gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space4 },
  readOnly: { flexDirection: "row", flexWrap: "wrap", gap: space.space4, marginTop: space.space4 },
});
