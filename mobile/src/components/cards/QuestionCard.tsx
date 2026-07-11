// DESIGN_SYSTEM.md §6 QuestionCard: question body; one Button per option
// (secondary, label bodyBold + description sub) stacked; free-text row when
// `question_allow_other` or no options.
//
// ARCHITECTURE.md §3 prompt_seq discipline: options answer the 1-based option
// number as a STRING via `answer{text, seq}`; buttons/input disable after the
// first tap/submit until a new `prompt_seq` arrives; never retry.
//
// HANDOFF(T3.3): ds/Button only supports a single-line label — it has no slot
// for the option's `description` sub-line, so option rows are a small local
// Pressable (Strike-driven, same visual language as Button's "secondary"
// variant) instead of reusing Button directly.
import { Send } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { type QuestionOption, type RemoteInput } from "../../lib/ws";
import { useStrike } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface QuestionCardProps {
  question: string;
  options: QuestionOption[];
  allowOther: boolean;
  promptSeq: number;
  send: (input: RemoteInput) => boolean;
  onQueueAnswer?: (input: Extract<RemoteInput, { kind: "allow" | "answer" }>) => void;
}

export function QuestionCard({ question, options, allowOther, promptSeq, send, onQueueAnswer }: QuestionCardProps) {
  const tokens = useTokens();
  const toast = useToast();
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);
  const [freeText, setFreeText] = useState("");
  const [queued, setQueued] = useState(false);

  useEffect(() => {
    setLockedSeq(null);
    setFreeText("");
    setQueued(false);
  }, [promptSeq]);

  const locked = lockedSeq === promptSeq;

  const answer = (text: string) => {
    if (locked || text.trim().length === 0) return;
    setLockedSeq(promptSeq);
    haptics.select();
    if (!send({ kind: "answer", text, seq: promptSeq })) {
      if (onQueueAnswer) { onQueueAnswer({ kind: "answer", text, seq: promptSeq }); setQueued(true); }
      else { setLockedSeq(null); toast.show("not sent — reconnect and try again", { tone: "danger" }); }
      haptics.mergeConflict();
    }
  };

  const showFreeText = allowOther || options.length === 0;

  return (
    <View style={styles.container}>
      <Text style={[typeScale.body, { color: tokens.ink }, styles.question]}>{question}</Text>
      {queued ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>will send on reconnect</Text> : null}

      {options.map((opt, idx) => (
        <OptionRow key={idx} option={opt} disabled={locked} onPress={() => answer(String(idx + 1))} />
      ))}

      {showFreeText ? (
        <View style={styles.freeTextRow}>
          <Input
            value={freeText}
            onChangeText={setFreeText}
            placeholder="type an answer…"
            editable={!locked}
            onSubmitEditing={() => answer(freeText)}
            returnKeyType="send"
            containerStyle={styles.freeTextInput}
            accessibilityLabel="free-text answer"
          />
          <IconButton
            icon={<Send size={20} strokeWidth={1.75} color={tokens.ink} />}
            onPress={() => answer(freeText)}
            disabled={locked || freeText.trim().length === 0}
            accessibilityLabel="send answer"
          />
        </View>
      ) : null}
    </View>
  );
}

function OptionRow({
  option,
  disabled,
  onPress,
}: {
  option: QuestionOption;
  disabled: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const strike = useStrike();

  return (
    <Animated.View style={strike.style}>
      <Pressable
        onPress={disabled ? undefined : onPress}
        onPressIn={disabled ? undefined : strike.onPressIn}
        onPressOut={disabled ? undefined : strike.onPressOut}
        disabled={disabled}
        accessibilityRole="button"
        accessibilityLabel={option.description ? `${option.label} — ${option.description}` : option.label}
        accessibilityState={{ disabled }}
        style={[
          styles.option,
          { backgroundColor: tokens.bg3, borderRadius: radii.radius8, opacity: disabled ? 0.4 : 1 },
        ]}
      >
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{option.label}</Text>
        {option.description ? (
          <Text style={[typeScale.sub, { color: tokens.ink2 }, styles.optionDetail]}>{option.description}</Text>
        ) : null}
      </Pressable>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  container: { gap: space.space8 },
  question: { marginBottom: space.space4 },
  option: { paddingHorizontal: space.space12, paddingVertical: space.space12, minHeight: 44 },
  optionDetail: { marginTop: space.space2 },
  freeTextRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space4 },
  freeTextInput: { flex: 1 },
});
