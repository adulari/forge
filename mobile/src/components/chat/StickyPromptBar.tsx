// A one-line bar over the top of the transcript quoting the prompt the visible turn answers; a
// press scrolls back to that prompt. The app's version of the TUI's "previous message" pill.
import { CornerLeftUp } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text } from "react-native";

import { promptPreview } from "../../lib/stickyPrompt";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { MiniPressable } from "./MiniPressable";

export interface StickyPromptBarProps {
  text: string;
  onPress: () => void;
}

export function StickyPromptBar({ text, onPress }: StickyPromptBarProps) {
  const tokens = useTokens();
  return (
    <MiniPressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`Your prompt: ${promptPreview(text, 200)}. Scroll to it`}
      wash={false}
      style={[styles.bar, { backgroundColor: tokens.bg2, borderBottomColor: tokens.border }]}
      testID="sticky-prompt-bar"
    >
      <CornerLeftUp size={13} strokeWidth={1.75} color={tokens.ink3} />
      <Text style={[typeScale.sub, styles.text, { color: tokens.ink2 }]} numberOfLines={1}>
        {promptPreview(text)}
      </Text>
    </MiniPressable>
  );
}

const styles = StyleSheet.create({
  bar: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    zIndex: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space16,
    minHeight: 34,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  text: { flex: 1, minWidth: 0 },
});
