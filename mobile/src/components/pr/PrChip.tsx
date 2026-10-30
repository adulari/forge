import { GitPullRequest } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text } from "react-native";

import { usePrStatus } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums } from "../../theme/typography";
import { prChip, type PrTone } from "./prModel";

export function toneColor(tone: PrTone, tokens: ReturnType<typeof useTokens>): string {
  switch (tone) {
    case "success":
      return tokens.success;
    case "danger":
      return tokens.danger;
    case "warn":
      return tokens.warn;
    case "accent":
      return tokens.accent;
    default:
      return tokens.ink3;
  }
}

/** The session header's PR chip: renders nothing until the branch has a pull request. */
export function PrChip({ sessionId, onPress }: { sessionId: string; onPress: () => void }) {
  const tokens = useTokens();
  const status = usePrStatus(sessionId);
  const pr = status.data?.pr;
  if (!pr) return null;
  const chip = prChip(pr);
  const color = toneColor(chip.tone, tokens);
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`Pull request ${chip.label}: ${chip.detail}`}
      style={({ pressed }) => [
        styles.chip,
        { borderColor: hexToRgba(color, 0.45), backgroundColor: hexToRgba(color, pressed ? 0.18 : 0.1) },
      ]}
    >
      <GitPullRequest size={13} strokeWidth={1.9} color={color} />
      <Text style={[styles.label, tabularNums, { color }]} numberOfLines={1}>
        {chip.label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: space.space8,
    height: 24,
    flexShrink: 0,
  },
  label: { fontFamily: monoFamily.regular, fontSize: 11.5 },
});
