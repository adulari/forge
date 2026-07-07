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
import { AttachmentRow } from "./Attachments";
import type { SentAttachment } from "./attach";
import { Markdown } from "./Markdown";
import { ReasoningDisclosure } from "./ReasoningDisclosure";

export interface MessageRowProps {
  row: HistoryRow;
  /**
   * Client-local attachments for the optimistic "just sent" bubble only — the daemon never
   * persists attachment metadata into `HistoryRow.content` (remote.rs: "the persisted row
   * stays text-only, images are transient input"), so a reloaded history row can't carry this.
   */
  attachments?: SentAttachment[];
}

// A text-file upload rides the NEXT prompt as a `@<path>` mention the daemon itself prepends
// server-side (forge-cli run.rs `prepend_attach_mentions`) — e.g.
// "@/tmp/.forge/uploads/<session>/1699999999999-notes.txt\n<the typed text>". That's the ONLY
// attachment trace a reloaded history row can carry (images leave none at all — see the
// `attachments` doc above), so it's parsed back out here into a file chip. Requiring
// `.forge/uploads/` in the path keeps this from ever misfiring on a real `@mention` a user typed.
const ATTACH_MENTION_RE = /^((?:@\S*\.forge\/uploads\/\S+ ?)+)\n([\s\S]*)$/;

function mentionsFromContent(content: string): { text: string; files: string[] } {
  const m = content.match(ATTACH_MENTION_RE);
  if (!m) return { text: content, files: [] };
  const files = m[1]
    .trim()
    .split(/\s+/)
    .map((tok) => tok.slice(1).split("/").pop() ?? tok)
    .map((name) => name.replace(/^\d+-/, ""));
  return { text: m[2], files };
}

function MessageRowImpl({ row, attachments }: MessageRowProps) {
  const tokens = useTokens();
  const isUser = row.role === "user";
  const isSystem = row.role === "system";

  // Only assistant turns carry inline `<think>` reasoning; a past turn's reasoning renders
  // collapsed here too, so scrollback isn't full of expanded thinking logs.
  const parsed = isUser || isSystem ? null : parseReasoning(row.content);
  const { text: userText, files: mentionFiles } = isUser
    ? mentionsFromContent(row.content)
    : { text: row.content, files: [] as string[] };
  const historyFileAttachments: SentAttachment[] = mentionFiles.map((name, i) => ({
    id: `mention-${row.seq}-${i}`,
    name,
    image: false,
  }));

  return (
    <View style={[styles.row, isUser && styles.userRow]}>
      <View
        style={[
          styles.bubble,
          isUser ? { backgroundColor: tokens.bg3 } : { backgroundColor: "transparent" },
        ]}
      >
        {attachments && attachments.length > 0 ? <AttachmentRow attachments={attachments} /> : null}
        {historyFileAttachments.length > 0 ? (
          <AttachmentRow attachments={historyFileAttachments} />
        ) : null}
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
          <Markdown content={userText} />
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
