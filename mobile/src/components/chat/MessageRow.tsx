// DESIGN_SYSTEM.md §6 — timeline rows for user/assistant/tool (system) content: Markdown +
// CodeBlock body for user/assistant; system rows (tool/diff-ish output) render compact mono
// per the same materials CodeBlock uses, without a full Markdown pass over structured text.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { HistoryRow } from "../../lib/api";
import { parseReasoning } from "../../lib/reasoning";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { Markdown } from "./Markdown";
import { ReasoningDisclosure } from "./ReasoningDisclosure";

export interface MessageRowProps {
  row: HistoryRow;
}

function MessageRowImpl({ row }: MessageRowProps) {
  const tokens = useTokens();
  const isUser = row.role === "user";
  const isSystem = row.role === "system";

  // Only assistant turns carry inline `<think>` reasoning; a past turn's reasoning renders
  // collapsed here too, so scrollback isn't full of expanded thinking logs.
  const parsed = isUser || isSystem ? null : parseReasoning(row.content);

  return (
    <View style={[styles.row, isUser && styles.userRow]}>
      <View
        style={[
          styles.bubble,
          isUser ? { backgroundColor: tokens.bg3 } : { backgroundColor: "transparent" },
        ]}
      >
        {isSystem ? (
          <Text
            style={[type.codeSmall, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
            selectable
          >
            {row.content}
          </Text>
        ) : parsed ? (
          <>
            {parsed.reasoning ? <ReasoningDisclosure reasoning={parsed.reasoning} /> : null}
            <Markdown content={parsed.answer} />
          </>
        ) : (
          <Markdown content={row.content} />
        )}
        {row.model ? (
          <Text style={[type.meta, styles.meta, { color: tokens.ink3 }]}>{row.model}</Text>
        ) : null}
      </View>
    </View>
  );
}

export const MessageRow = React.memo(MessageRowImpl);

const styles = StyleSheet.create({
  row: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
  userRow: { alignItems: "flex-end" },
  bubble: { borderRadius: 12, paddingHorizontal: space.space12, paddingVertical: space.space8, maxWidth: "92%" },
  meta: { marginTop: space.space4 },
});
