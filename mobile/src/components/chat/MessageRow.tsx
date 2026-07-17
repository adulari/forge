// DESIGN_SYSTEM.md §6 — timeline rows for user/assistant/tool (system) content: Markdown +
// CodeBlock body for user/assistant; system rows (tool/diff-ish output) render compact mono
// per the same materials CodeBlock uses, without a full Markdown pass over structured text.
import * as Clipboard from "expo-clipboard";
import { Copy } from "lucide-react-native";
import React from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { HistoryRow } from "../../lib/api";
import { parseReasoning } from "../../lib/reasoning";
import { haptics } from "../../lib/haptics";
import { useSessionCtx } from "../../lib/sessionContext";
import { useForgeline } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import { AttachmentRow } from "./Attachments";
import type { SentAttachment } from "./attach";
import { Markdown } from "./Markdown";
import { ReasoningDisclosure } from "./ReasoningDisclosure";
import { SystemOutput } from "./SystemOutput";

const IS_WEB = Platform.OS === "web";

export interface MessageRowProps {
  row: HistoryRow;
  onLongPress?: (message: HistoryRow) => void;
  /**
   * Client-local attachments for the optimistic "just sent" bubble only — the daemon never
   * persists attachment metadata into `HistoryRow.content` (remote.rs: "the persisted row
   * stays text-only, images are transient input"), so a reloaded history row can't carry this.
   */
  attachments?: SentAttachment[];
}

// A file upload rides the NEXT prompt as a `@<path>` mention the daemon itself prepends
// server-side (forge-cli run.rs `prepend_attach_mentions`) — e.g.
// "@/tmp/.forge/uploads/<session>/1699999999999-notes.txt\n<the typed text>". That's the ONLY
// attachment trace a reloaded history row can carry, so it's parsed back out here. Requiring
// `.forge/uploads/` in the path keeps this from ever misfiring on a real `@mention` a user typed.
// Images ride the same `@path` convention (server-side extension set mirrored in
// `crates/forge-cli/src/image_input.rs`) — those render as an inline thumbnail (fetched from
// `GET /api/upload`) instead of a file chip; everything else stays a file chip as before.
const ATTACH_MENTION_RE = /^((?:@\S*\.forge\/uploads\/\S+ ?)+)\n([\s\S]*)$/;
const IMAGE_MENTION_RE = /\.(png|jpe?g|gif|webp|bmp)$/i;
// Same convention as ATTACH_MENTION_RE, but not anchored to the trailing `\n<text>` — a
// `PastSessionRow.preview` (History tab) can be server-truncated mid-mention and never carry
// the full match, so it needs a lenient leading-strip instead of the exact chat-transcript parse.
const LEADING_MENTION_RE = /^((?:@\S*\.forge\/uploads\/\S+\s*)+)/;

/** Strips a leading upload-mention prefix from preview text, replacing an attachment-only
 * preview with a human-readable placeholder instead of leaking the raw `.forge/uploads/` path. */
export function stripLeadingAttachMentions(text: string): string {
  const m = text.match(LEADING_MENTION_RE);
  if (!m) return text;
  const rest = text.slice(m[0].length).trim();
  if (rest) return rest;
  const hasImage = m[1].trim().split(/\s+/).some((tok) => IMAGE_MENTION_RE.test(tok));
  return hasImage ? "[image attached]" : "[file attached]";
}

interface ImageMention {
  name: string;
  path: string;
}

function mentionsFromContent(content: string): {
  text: string;
  files: string[];
  images: ImageMention[];
} {
  const m = content.match(ATTACH_MENTION_RE);
  if (!m) return { text: content, files: [], images: [] };
  const files: string[] = [];
  const images: ImageMention[] = [];
  for (const tok of m[1].trim().split(/\s+/)) {
    const path = tok.slice(1);
    const name = (path.split("/").pop() ?? tok).replace(/^\d+-/, "");
    if (IMAGE_MENTION_RE.test(path)) {
      images.push({ name, path });
    } else {
      files.push(name);
    }
  }
  return { text: m[2], files, images };
}

export function displayMessageText(row: HistoryRow): string {
  if (row.role === "user") return mentionsFromContent(row.content).text;
  if (row.role === "assistant") return parseReasoning(row.content).answer;
  return row.content;
}

function MessageRowImpl({ row, attachments, onLongPress }: MessageRowProps) {
  const tokens = useTokens();
  const entrance = useForgeline(Math.max(0, row.seq));
  const toast = useToast();
  const { baseUrl, sessionId } = useSessionCtx();
  const isUser = row.role === "user";
  const isSystem = row.role === "system";

  // Only assistant turns carry inline `<think>` reasoning; a past turn's reasoning renders
  // collapsed here too, so scrollback isn't full of expanded thinking logs.
  const parsed = isUser || isSystem ? null : parseReasoning(row.content);
  const {
    text: userText,
    files: mentionFiles,
    images: mentionImages,
  } = isUser
    ? mentionsFromContent(row.content)
    : { text: row.content, files: [] as string[], images: [] as ImageMention[] };
  const historyFileAttachments: SentAttachment[] = [
    ...mentionImages.map((img, i) => ({
      id: `mention-img-${row.seq}-${i}`,
      name: img.name,
      image: true,
      uri: baseUrl
        ? `${baseUrl}/api/upload?session=${encodeURIComponent(sessionId)}&path=${encodeURIComponent(img.path)}`
        : undefined,
    })),
    ...mentionFiles.map((name, i) => ({
      id: `mention-${row.seq}-${i}`,
      name,
      image: false,
    })),
  ];

  // Per-block `selectable` Text (Markdown.tsx) can't drag-select across paragraphs, and there
  // was no way to grab a whole reply at once — one tap/long-press now copies the full row:
  // the parsed answer (no `<think>` block) for assistant turns, else the plain row text.
  const copyText = displayMessageText(row);
  const onCopyRow = async () => {
    await Clipboard.setStringAsync(copyText);
    toast.show("message copied");
  };

  const handleLongPress = () => {
    if (!onLongPress || IS_WEB) return;
    haptics.select();
    onLongPress(row);
  };

  return (
    <Animated.View style={[styles.row, !isUser && !isSystem && styles.assistantRow, isUser && styles.userRow, entrance]}>
      {!isUser && !isSystem ? <View style={[styles.spine, { backgroundColor: tokens.border }]} /> : null}
      <Pressable
        onLongPress={onLongPress && !IS_WEB ? handleLongPress : undefined}
        style={[
          styles.bubble,
          isUser
            ? [styles.userBubble, { backgroundColor: tokens.bg2, borderColor: tokens.border }]
            : { backgroundColor: "transparent" },
        ]}
      >
        {attachments && attachments.length > 0 ? <AttachmentRow attachments={attachments} /> : null}
        {historyFileAttachments.length > 0 ? (
          <AttachmentRow attachments={historyFileAttachments} />
        ) : null}
        {isSystem ? (
          <SystemOutput content={row.content} />
        ) : parsed ? (
          <>
            {parsed.reasoning ? <ReasoningDisclosure reasoning={parsed.reasoning} /> : null}
            <Markdown content={parsed.answer} />
          </>
        ) : (
          <Markdown content={userText} />
        )}
        {row.model || (IS_WEB && !isSystem) ? (
          <View style={styles.metaRow}>
            {row.model ? <Text style={[type.meta, styles.model, { color: tokens.ink3 }]} numberOfLines={1}>{row.model}</Text> : null}
            {IS_WEB && !isSystem ? (
              <IconButton
                accessibilityLabel="copy message"
                onPress={onCopyRow}
                icon={<Copy size={16} strokeWidth={1.75} color={tokens.ink3} />}
              />
            ) : null}
          </View>
        ) : null}
      </Pressable>
    </Animated.View>
  );
}

export const MessageRow = React.memo(MessageRowImpl);

const styles = StyleSheet.create({
  row: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
  assistantRow: { paddingLeft: space.space24, position: "relative" },
  spine: { position: "absolute", left: space.space16, top: space.space8, bottom: space.space8, width: 2, borderRadius: radii.radiusPill },
  userRow: { alignItems: "flex-end" },
  bubble: { borderRadius: 12, paddingHorizontal: space.space12, paddingVertical: space.space8 },
  userBubble: { maxWidth: "85%", borderRadius: radii.radius16, borderWidth: StyleSheet.hairlineWidth },
  metaRow: { flexDirection: "row", alignItems: "center", gap: space.space4, marginTop: space.space4 },
  model: { flex: 1 },
});
