// Question/answer card (BUILD_PLAN §1.3 / §6 Chat action cards, Batch 3).
//
// Renders `snapshot.question` + `question_options` as tappable choices, mirroring the web
// control page's question block (crates/forge-cli/src/remote_assets/app.js `renderActions`
// lines 861-875 + styles.css lines 55-61): an accent-bordered `.prompt` card with a "❓"
// prefix, options stacked as buttons (bold label + dim description), and a free-text row
// shown whenever there are no options OR `question_allow_other` is set (matches the web
// condition `!opts.length || s.question_allow_other` verbatim). Options are additionally
// numbered here (BUILD_PLAN Batch 3 spec) for faster scanning on a touch target — the web
// card relies on button order alone. Sends `{kind:"answer", text, seq}` where `text` is the
// 1-based option index as a string, or the typed free text.
//
// prompt_seq discipline (UI_RULES.md #16): mirrors permissionCard.tsx — disable immediately
// after the first answer, re-enable only when the server serves a genuinely new prompt_seq.
import React, { useCallback, useEffect, useState } from "react";
import { Platform, Pressable, Text, TextInput, View } from "react-native";
import Animated from "react-native-reanimated";

import { theme } from "../lib/theme";
import { usePressScale } from "../lib/motion";
import type { QuestionOption, RemoteInput } from "../lib/ws";
import { PrimaryButton } from "./ui";

export interface QuestionCardProps {
  question: string;
  options: QuestionOption[];
  allowOther: boolean;
  seq: number;
  send: (input: RemoteInput) => void;
}

function lightHaptic() {
  if (Platform.OS === "web") return;
  import("expo-haptics")
    .then((H) => H.impactAsync(H.ImpactFeedbackStyle.Light))
    .catch(() => {});
}

function OptionButton({
  index,
  option,
  disabled,
  onPress,
}: {
  index: number;
  option: QuestionOption;
  disabled: boolean;
  onPress: () => void;
}) {
  const { style, onPressIn, onPressOut } = usePressScale();
  return (
    <Animated.View style={style}>
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        disabled={disabled}
        className={`flex-row items-start gap-8 bg-chipBg border border-border rounded-md px-10 py-10 ${
          disabled ? "opacity-50" : ""
        }`}
        style={{ minHeight: 44 }}
      >
        <Text className="text-dim text-[13px] font-semibold">{index + 1}.</Text>
        <View className="flex-1 gap-2">
          <Text className="text-accent text-[14px] font-bold">{option.label}</Text>
          {option.description ? (
            <Text className="text-dim text-[12px]">{option.description}</Text>
          ) : null}
        </View>
      </Pressable>
    </Animated.View>
  );
}

function FreeTextInput({
  value,
  onChangeText,
  disabled,
  onSubmit,
}: {
  value: string;
  onChangeText: (v: string) => void;
  disabled: boolean;
  onSubmit: () => void;
}) {
  return (
    <TextInput
      value={value}
      onChangeText={onChangeText}
      placeholder="answer…"
      placeholderTextColor={theme.colors.dim}
      editable={!disabled}
      returnKeyType="done"
      onSubmitEditing={onSubmit}
      className="flex-1 bg-panelDeep border border-border rounded-md px-10 text-ink text-[15px]"
      style={{ minHeight: 44 }}
    />
  );
}

export function QuestionCard({ question, options, allowOther, seq, send }: QuestionCardProps) {
  const [answeredSeq, setAnsweredSeq] = useState<number | null>(null);
  const [freeText, setFreeText] = useState("");

  useEffect(() => {
    setAnsweredSeq((prev) => (prev !== null && prev !== seq ? null : prev));
    setFreeText("");
  }, [seq]);

  const disabled = answeredSeq === seq;
  const showFreeText = options.length === 0 || allowOther;

  const pickOption = useCallback(
    (index: number) => {
      if (disabled) return;
      lightHaptic();
      setAnsweredSeq(seq);
      send({ kind: "answer", text: String(index + 1), seq });
    },
    [disabled, seq, send],
  );

  const submitFreeText = useCallback(() => {
    const trimmed = freeText.trim();
    if (!trimmed || disabled) return;
    lightHaptic();
    setAnsweredSeq(seq);
    send({ kind: "answer", text: trimmed, seq });
  }, [freeText, disabled, seq, send]);

  return (
    <View className="bg-panel border border-accent rounded-lg px-10 py-10 gap-8">
      <View className="flex-row items-start gap-6">
        <Text className="text-accent text-[15px] font-bold">❓</Text>
        <Text className="flex-1 text-accent text-[15px] font-bold">{question}</Text>
      </View>

      {options.length ? (
        <View className="gap-6">
          {options.map((opt, i) => (
            <OptionButton
              key={i}
              index={i}
              option={opt}
              disabled={disabled}
              onPress={() => pickOption(i)}
            />
          ))}
        </View>
      ) : null}

      {showFreeText ? (
        <View className="flex-row items-center gap-8">
          <FreeTextInput
            value={freeText}
            onChangeText={setFreeText}
            disabled={disabled}
            onSubmit={submitFreeText}
          />
          <PrimaryButton
            label="Answer"
            onPress={submitFreeText}
            disabled={disabled || !freeText.trim()}
            fullWidth={false}
          />
        </View>
      ) : null}
    </View>
  );
}
