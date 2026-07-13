import { ArrowLeft } from "lucide-react-native";
import { router } from "expo-router";
import React from "react";
import { Pressable, StyleSheet, Text } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";

export function BackLink({ label = "Settings", onPress }: { label?: string; onPress?: () => void }) {
  const tokens = useTokens();
  return (
    <Pressable onPress={onPress ?? (() => router.back())} accessibilityRole="button" accessibilityLabel={`Back to ${label}`} style={styles.button}>
      <ArrowLeft size={18} strokeWidth={1.75} color={tokens.accent} />
      <Text style={[type.bodyBold, { color: tokens.accent }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: { alignSelf: "flex-start", minHeight: tapTarget, flexDirection: "row", alignItems: "center", gap: space.space4, paddingRight: space.space12 },
});
