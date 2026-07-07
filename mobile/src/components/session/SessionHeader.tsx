// Session shell header (T3.1, DESIGN_SYSTEM.md §6): back control, title, cwd (mono,
// head-ellipsized), worktree Badge, exposure Badge. The danger Banner for public exposure
// is rendered by the shell itself (_layout.tsx) — this component owns the header row only.
import { ArrowLeft } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { IconButton } from "../ds/IconButton";

export interface SessionHeaderProps {
  title: string;
  cwd: string;
  worktree: string | null;
  exposure: string;
  onBack: () => void;
}

export function SessionHeader({ title, cwd, worktree, exposure, onBack }: SessionHeaderProps) {
  const tokens = useTokens();
  const isPublic = exposure.startsWith("public");

  return (
    <View style={styles.wrap}>
      <View style={styles.row}>
        <IconButton
          icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onBack}
          accessibilityLabel="Back"
        />
        <Text style={[typeScale.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          {title}
        </Text>
        {worktree ? <Badge label="worktree" tone="outline" /> : null}
        <Badge label={exposure} tone={isPublic ? "danger" : "neutral"} />
      </View>
      <Text
        style={[typeScale.codeSmall, styles.cwd, { color: tokens.ink2 }]}
        numberOfLines={1}
        ellipsizeMode="head"
      >
        {cwd}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space4 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  // Aligns under the title text, past the 44pt back-button hit area.
  cwd: { paddingLeft: 44 + space.space8 },
});
