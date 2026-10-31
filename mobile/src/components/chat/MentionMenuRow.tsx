// One row of the composer's `@` menu: a workspace file hit, or a context mention
// (`@session:<name>`, `@terminal`, `@browser`, `@Subagents`).
import { FileCode2 } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text } from "react-native";

import type { WorkspaceSearchResult } from "../../lib/api";
import { isMentionOption, type MentionOption } from "../../lib/mentionsModel";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { workspaceBasename } from "../workspace/workspaceModel";
import { MentionIcon } from "./MentionChips";

interface MentionMenuRowProps {
  result: WorkspaceSearchResult | MentionOption;
  selected: boolean;
  onSelect: (path: string) => void;
}

export function MentionMenuRow({ result, selected, onSelect }: MentionMenuRowProps) {
  const tokens = useTokens();
  const context = isMentionOption(result) ? result : null;
  const iconColor = selected ? tokens.accent : tokens.ink3;
  return (
    <Pressable
      onPress={() => onSelect(result.path)}
      accessibilityRole="menuitem"
      accessibilityLabel={`Mention ${context?.label ?? result.path}`}
      accessibilityState={{ selected }}
      style={[styles.row, { backgroundColor: selected ? tokens.selection : "transparent" }]}
    >
      {context ? (
        <MentionIcon kind={context.kind} size={15} color={iconColor} />
      ) : (
        <FileCode2 size={15} strokeWidth={1.75} color={iconColor} />
      )}
      <Text style={[type.body, styles.name, { color: tokens.ink }]} numberOfLines={1}>
        {context ? context.label : workspaceBasename(result.path)}
      </Text>
      <Text style={[type.monoMeta, styles.detail, { color: tokens.ink3 }]} numberOfLines={1}>
        {context ? context.detail : result.path}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: {
    minHeight: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    paddingVertical: 6,
    borderRadius: radii.radius4,
  },
  name: { flexShrink: 0, maxWidth: "38%" },
  detail: { flex: 1 },
});
