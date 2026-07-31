import { MessageSquarePlus } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import {
  addReviewComment,
  reviewRangeLabel,
  type ReviewCommentSource,
  type ReviewLineSelection,
} from "../../lib/reviewComments";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export interface ReviewCommentSheetProps {
  visible: boolean;
  sessionId: string;
  path: string;
  revision: string;
  source?: ReviewCommentSource;
  staged: boolean;
  selection: ReviewLineSelection | null;
  onClose: () => void;
  onAdded: () => void;
}

export function ReviewCommentSheet({
  visible,
  sessionId,
  path,
  revision,
  source = "working-tree",
  staged,
  selection,
  onClose,
  onAdded,
}: ReviewCommentSheetProps): React.JSX.Element {
  const tokens = useTokens();
  const [text, setText] = useState("");

  useEffect(() => {
    if (!visible) setText("");
  }, [visible]);

  const submit = () => {
    const comment = text.trim();
    if (!selection || !comment) return;
    addReviewComment({
      sessionId,
      source,
      path,
      revision,
      staged,
      side: selection.side,
      startLine: selection.startLine,
      endLine: selection.endLine,
      lines: selection.lines,
      text: comment,
    });
    setText("");
    onAdded();
    onClose();
  };

  const range = selection
    ? reviewRangeLabel(selection.side, selection.startLine, selection.endLine)
    : "no lines selected";

  return (
    <Sheet
      visible={visible}
      onClose={onClose}
      accessibilityLabel="Add review annotation"
      snapPoints={[1]}
      maxHeightRatio={0.76}
    >
      <View style={styles.body}>
        <View style={styles.heading}>
          <View style={styles.headingCopy}>
            <Text style={[typeScale.heading, { color: tokens.ink }]}>Review annotation</Text>
            <Text
              style={[typeScale.monoMeta, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
              numberOfLines={1}
            >
              {path} · {range} ·{" "}
              {source === "turn"
                ? "turn diff"
                : source === "fork"
                  ? "fork diff"
                  : staged
                    ? "staged"
                    : "working tree"}
            </Text>
          </View>
          <MessageSquarePlus size={20} strokeWidth={1.7} color={tokens.accent} />
        </View>

        <ScrollView
          horizontal
          style={[
            styles.context,
            {
              backgroundColor: tokens.bg0,
              borderColor: tokens.border,
              borderRadius: radii.radius8,
            },
          ]}
          contentContainerStyle={styles.contextBody}
        >
          <View>
            {(selection?.lines ?? []).map((line) => {
              const gutter = line.kind === "add" ? "+" : line.kind === "del" ? "−" : " ";
              const color =
                line.kind === "add"
                  ? tokens.success
                  : line.kind === "del"
                    ? tokens.danger
                    : tokens.ink3;
              return (
                <Text
                  key={`${line.kind}:${line.lineNo}`}
                  style={[typeScale.codeSmall, { color, fontFamily: monoFamily.regular }]}
                >
                  {line.lineNo.toString().padStart(4, " ")} {gutter} {line.text || " "}
                </Text>
              );
            })}
          </View>
        </ScrollView>

        <Input
          label="Comment"
          value={text}
          onChangeText={setText}
          multiline
          numberOfLines={4}
          textAlignVertical="top"
          placeholder="What should change, and why?"
          autoCapitalize="sentences"
          clearable
          containerStyle={styles.input}
          accessibilityLabel="Review comment"
        />
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          This is attached to your next prompt. Tap its composer chip to remove it before sending.
        </Text>
        <Button
          label="Add to next prompt"
          onPress={submit}
          disabled={!selection || text.trim().length === 0}
          fullWidth
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: {
    paddingHorizontal: space.space16,
    paddingBottom: space.space16,
    gap: space.space12,
  },
  heading: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  headingCopy: { flex: 1, gap: 2 },
  context: { maxHeight: 150, borderWidth: StyleSheet.hairlineWidth },
  contextBody: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
  input: { minHeight: 112 },
});
