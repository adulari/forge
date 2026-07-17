// DESIGN_SYSTEM.md §6 — timeline rows for user/assistant/tool (system) content: Markdown +
// CodeBlock body for user/assistant; system rows (tool/diff-ish output) render compact mono
// per the same materials CodeBlock uses, without a full Markdown pass over structured text.
import * as Clipboard from "expo-clipboard";
import { Copy, MoreHorizontal } from "lucide-react-native";
import React, { useState } from "react";
import { Platform, Pressable, StyleSheet, View } from "react-native";
import Animated from "react-native-reanimated";

import type { HistoryRow } from "../../lib/api";
import { parseReasoning } from "../../lib/reasoning";
import { haptics } from "../../lib/haptics";
import { useSessionCtx } from "../../lib/sessionContext";
import { useForgeline } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import { AttachmentRow } from "./Attachments";
import type { SentAttachment } from "./attach";
import { Markdown } from "./Markdown";
import { ReasoningDisclosure } from "./ReasoningDisclosure";
import { SystemOutput } from "./SystemOutput";

const IS_WEB = Platform.OS === "web";

// DOM hover passthrough (same cast pattern as DesktopWindowChrome's onDoubleClick):
// react-native-web's Pressable hover events proved unreliable on a handler-less wrapper,
// so the row listens to real mouseenter/mouseleave instead.
type HoverViewProps = import("react-native").ViewProps & {
  onMouseEnter?: () => void;
  onMouseLeave?: () => void;
};
const HoverView = View as unknown as React.ComponentType<HoverViewProps>;

export interface MessageRowProps {
  row: HistoryRow;
  /** Open the message-actions surface. `anchor` (viewport coords) is set for pointer-driven
   * opens (right-click, the hover ⋯ button) so desktop can show an anchored popover; absent
   * for long-press, which gets the bottom sheet. */
  onLongPress?: (message: HistoryRow, anchor?: { x: number; y: number }) => void;
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
  // Entrance only for rows that just arrived — virtualization remounts rows while
  // scrolling, and replaying the fade there made settled messages flicker transparent.
  // Lazy useState: evaluated once at mount (the sanctioned home for an impure read).
  const [isFresh] = useState(() => Date.now() / 1000 - row.created_at < 5);
  const entrance = useForgeline(Math.max(0, row.seq), isFresh);
  const toast = useToast();
  const { baseUrl, sessionId } = useSessionCtx();
  const isUser = row.role === "user";
  const isSystem = row.role === "system";
  // Hearth: no always-visible model attribution or copy icon on the row — copy is a
  // long-press affordance on native (MessageActionsSheet), and on web a hover affordance
  // scoped to the WHOLE row (a short bubble left dead zones the cursor crossed on its way
  // to the pill, un-hovering it mid-travel) with a short grace delay before hiding.
  const [hovered, setHovered] = useState(false);
  const hideTimer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const hoverIn = () => {
    if (hideTimer.current != null) clearTimeout(hideTimer.current);
    hideTimer.current = null;
    setHovered(true);
  };
  const hoverOut = () => {
    if (hideTimer.current != null) clearTimeout(hideTimer.current);
    hideTimer.current = setTimeout(() => setHovered(false), 200);
  };
  React.useEffect(
    () => () => {
      if (hideTimer.current != null) clearTimeout(hideTimer.current);
    },
    [],
  );

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

  // Long-press works on native AND touch-web; desktop web additionally gets right-click.
  const handleLongPress = () => {
    if (!onLongPress) return;
    haptics.select();
    onLongPress(row);
  };

  // RNW forwards DOM handlers it doesn't know about (same passthrough DesktopWindowChrome
  // uses for onDoubleClick) — typed via cast because react-native's Pressable props don't
  // model web-only events.
  const webContextMenu = IS_WEB && onLongPress
    ? ({
        onContextMenu: (e: { preventDefault: () => void; clientX: number; clientY: number }) => {
          e.preventDefault();
          onLongPress(row, { x: e.clientX, y: e.clientY });
        },
      } as Record<string, unknown>)
    : null;

  return (
    <Animated.View style={entrance}>
      <HoverView
        onMouseEnter={IS_WEB ? hoverIn : undefined}
        onMouseLeave={IS_WEB ? hoverOut : undefined}
        style={[styles.row, !isUser && !isSystem && styles.assistantRow, isUser && styles.userRow]}
      >
      {!isUser && !isSystem ? <View style={[styles.spine, { backgroundColor: tokens.border }]} /> : null}
      <Pressable
        onLongPress={onLongPress ? handleLongPress : undefined}
        {...webContextMenu}
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
        {/* Always mounted, opacity-toggled, INSIDE the bubble's bounds: mounting-on-hover
            plus a negative-offset position let the pill vanish before the cursor could
            reach it (leaving the bubble un-hovered it). Kept in-bounds the pill is part of
            the hover subtree, so moving onto it keeps the row hovered; opacity means zero
            layout shift and no mount/unmount flicker. */}
        {IS_WEB && !isSystem ? (
          <View
            style={[
              styles.hoverActions,
              webActionsTransition,
              {
                backgroundColor: tokens.bg2,
                borderColor: tokens.border,
                opacity: hovered ? 1 : 0,
                pointerEvents: hovered ? "auto" : "none",
              },
            ]}
          >
            <IconButton
              accessibilityLabel="copy message"
              onPress={onCopyRow}
              icon={<Copy size={14} strokeWidth={1.75} color={tokens.ink2} />}
            />
            {onLongPress ? (
              <IconButton
                accessibilityLabel="message actions"
                onPress={(e?: { nativeEvent?: { pageX?: number; pageY?: number } }) =>
                  onLongPress(row, {
                    x: e?.nativeEvent?.pageX ?? 0,
                    y: e?.nativeEvent?.pageY ?? 0,
                  })
                }
                icon={<MoreHorizontal size={14} strokeWidth={1.75} color={tokens.ink2} />}
              />
            ) : null}
          </View>
        ) : null}
      </Pressable>
      </HoverView>
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
  hoverActions: {
    position: "absolute",
    top: 4,
    right: 4,
    flexDirection: "row",
    borderRadius: radii.radius8,
    borderWidth: StyleSheet.hairlineWidth,
    zIndex: 2,
  },
});

// Web-only fade so the pill eases in instead of popping (same untyped-CSS cast pattern
// as Sheet.tsx's webTransition).
const webActionsTransition = IS_WEB
  ? ({
      transitionProperty: "opacity",
      transitionDuration: "120ms",
    } as unknown as import("react-native").ViewStyle)
  : null;
