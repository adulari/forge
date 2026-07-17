// Hearth "Duel" launcher — de-boxed sheet: type-first heading, one task field, accent start
// action, and (when a draft is in the composer) a hairline row to duel the current prompt
// instead. Sending fires the existing `/duel <task>` command; when both models finish, the
// server opens the winner picker (rendered by DuelView) — the footnote says so.
import { Swords } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function DuelSheet({
  visible,
  onClose,
  send,
  currentPrompt,
}: {
  visible: boolean;
  onClose: () => void;
  send: (input: RemoteInput) => boolean;
  /** The current composer draft, if any — enables the "duel the current prompt" row. */
  currentPrompt?: string;
}) {
  const tokens = useTokens();
  const [task, setTask] = useState("");

  const start = (text: string) => {
    const trimmed = text.trim();
    if (trimmed && send({ kind: "prompt", text: `/duel ${trimmed}` })) {
      setTask("");
      onClose();
    }
  };

  const draft = currentPrompt?.trim();

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Start model duel" snapPoints={[0.6]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Model duel</Text>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          Race Forge&apos;s best models on the same task in isolated worktrees, then merge the winning answer.
        </Text>

        <Input
          label="Task"
          value={task}
          onChangeText={setTask}
          placeholder="Explain the failover bug"
          multiline
          accessibilityLabel="Duel task"
          returnKeyType="send"
          onSubmitEditing={() => start(task)}
        />
        <Button
          label="Start duel"
          onPress={() => start(task)}
          disabled={!task.trim()}
          fullWidth
          icon={<Swords size={16} strokeWidth={1.75} color={task.trim() ? tokens.onAccent : tokens.ink4} />}
        />

        {draft ? (
          <>
            <Text style={[typeScale.section, styles.orLabel, { color: tokens.ink4 }]}>or</Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Duel the current prompt"
              onPress={() => start(draft)}
              style={[styles.draftRow, { borderTopColor: tokens.hairline }]}
            >
              <Text style={[typeScale.bodyBold, { color: tokens.accent }]}>Duel the current prompt</Text>
              <Text style={[styles.mono, { color: tokens.ink3 }]} numberOfLines={1}>
                {draft}
              </Text>
            </Pressable>
          </>
        ) : null}

        <Text style={[typeScale.meta, styles.footnote, { color: tokens.ink4 }]}>
          When both models finish, a picker opens to compare their answers side by side and pick the winner.
        </Text>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space32, gap: space.space12 },
  subtitle: { marginTop: 2, marginBottom: space.space4 },
  orLabel: { paddingTop: space.space4 },
  draftRow: { minHeight: 52, justifyContent: "center", gap: 2, borderTopWidth: StyleSheet.hairlineWidth },
  footnote: { paddingTop: space.space4 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11.5, lineHeight: 16 },
});
