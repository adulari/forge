// Inline comments pile up per agent as a review draft ("3 comments") and go out as one prompt
// ("Send review"), the way a GitHub review does. A single comment can also be sent at once.
import { ChevronDown, ChevronUp, MessageSquarePlus, Send, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, TextInput, View } from "react-native";

import {
  addReviewComment,
  clearReviewComments,
  removeReviewComment,
  reviewRangeLabel,
  useReviewComments,
  type ReviewComment,
  type ReviewLineSelection,
} from "../../lib/reviewComments";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor, webInputTextStyle } from "../../theme/typography";
import { useToast } from "../ds/ToastHost";
import { sendToAgent } from "./agentBridge";
import { useIde } from "./IdeContext";
import { useLatest } from "./useLatest";
import { onIdeSendReview } from "./ideEvents";
import { buildReviewPrompt, buildSingleCommentPrompt, draftLabel, groupDraftByFile } from "./reviewDraft";
import { IdeIconButton, IdeTextButton, Keycap } from "./ideUi";

/** Send text to an agent. If its panel is not showing yet, show it and let the message wait. */
export function useAgentSend(): (sessionId: string, text: string) => "sent" | "queued" {
  const { panelSessionId, setScope } = useIde();
  return useCallback(
    (sessionId, text) => {
      if (panelSessionId !== sessionId) setScope(sessionId);
      return sendToAgent(sessionId, text);
    },
    [panelSessionId, setScope],
  );
}

export interface CommentTarget {
  selection: ReviewLineSelection;
  revision: string;
}

export function CommentComposer({
  sessionId,
  path,
  staged,
  target,
  onDone,
}: {
  sessionId: string;
  path: string;
  staged: boolean;
  target: CommentTarget;
  onDone: () => void;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const send = useAgentSend();
  const [text, setText] = useState("");
  const { selection } = target;
  const make = (): ReviewComment | null => {
    const body = text.trim();
    if (!body) return null;
    return {
      id: "draft",
      sessionId,
      source: "working-tree",
      path,
      revision: target.revision,
      staged,
      side: selection.side,
      startLine: selection.startLine,
      endLine: selection.endLine,
      lines: selection.lines,
      text: body,
    };
  };
  const add = () => {
    const comment = make();
    if (!comment) return;
    const { id: _id, sessionId: _session, ...rest } = comment;
    addReviewComment({ ...rest, sessionId });
    onDone();
  };
  const sendNow = () => {
    const comment = make();
    if (!comment) return;
    const where = send(sessionId, buildSingleCommentPrompt(comment));
    toast.show(where === "sent" ? "Comment sent to the agent" : "Comment will send when the agent opens", { tone: "neutral" });
    onDone();
  };
  const keys = {
    onKeyDown: (event: { key?: string; metaKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean; preventDefault?: () => void; stopPropagation?: () => void }) => {
      event.stopPropagation?.();
      if (event.key === "Escape") {
        event.preventDefault?.();
        onDone();
      } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
        event.preventDefault?.();
        if (event.shiftKey) add();
        else sendNow();
      }
    },
  } as object;
  const quote = selection.lines.slice(0, 3);
  return (
    <View style={[styles.composer, { borderTopColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <Text style={[styles.label, { color: tokens.ink3 }]} numberOfLines={1}>
        {`Comment on ${path.slice(path.lastIndexOf("/") + 1)} · ${reviewRangeLabel(selection.side, selection.startLine, selection.endLine)}`}
      </Text>
      {quote.map((line) => (
        <Text key={line.lineNo} numberOfLines={1} style={[styles.quote, { color: tokens.ink3 }]}>
          {`${line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "} ${line.text}`}
        </Text>
      ))}
      <TextInput
        value={text}
        onChangeText={setText}
        autoFocus
        multiline
        placeholder="What should the agent change?"
        placeholderTextColor={tokens.ink4}
        accessibilityLabel="Review comment"
        {...keys}
        style={[styles.input, webInputTextStyle, { color: tokens.ink, borderColor: tokens.borderStrong, backgroundColor: tokens.bg0 }]}
      />
      <View style={styles.actions}>
        <IdeTextButton label="Add to review" icon={<MessageSquarePlus size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={add} disabled={!text.trim()} keyHint="⇧⌘↵" />
        <IdeTextButton label="Send now" icon={<Send size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={sendNow} disabled={!text.trim()} keyHint="⌘↵" />
        <View style={styles.grow} />
        <IdeTextButton label="Cancel" onPress={onDone} keyHint="Esc" />
      </View>
    </View>
  );
}

/** The draft bar: how many comments are waiting, and the one button that sends them all. */
export function ReviewDraftBar({ sessionId }: { sessionId: string | null }) {
  if (!sessionId) return null;
  return <Bar sessionId={sessionId} />;
}

function Bar({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const send = useAgentSend();
  const { openChange, sessions } = useIde();
  const comments = useReviewComments(sessionId);
  const [open, setOpen] = useState(false);
  const agent = sessions.find((row) => row.id === sessionId);
  const sendReview = () => {
    if (comments.length === 0) return;
    const where = send(sessionId, buildReviewPrompt(comments));
    clearReviewComments(sessionId);
    setOpen(false);
    toast.show(where === "sent" ? `Review sent to ${agent?.title?.trim() || "the agent"}` : "Review will send when the agent opens", { tone: "neutral" });
  };
  const sendRef = useLatest(sendReview);
  const has = comments.length > 0;
  useEffect(() => (has ? onIdeSendReview(() => sendRef.current()) : undefined), [has, sendRef]);
  if (!has) return null;
  return (
    <View style={[styles.bar, { borderTopColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <View style={styles.barRow}>
        <IdeIconButton
          icon={open ? <ChevronDown size={13} strokeWidth={1.75} color={tokens.ink2} /> : <ChevronUp size={13} strokeWidth={1.75} color={tokens.ink2} />}
          label={open ? "Hide comments" : "Show comments"}
          onPress={() => setOpen((value) => !value)}
        />
        <Text style={[styles.count, { color: tokens.ink }]}>{draftLabel(comments.length)}</Text>
        <Text numberOfLines={1} style={[styles.sub, { color: tokens.ink3 }]}>
          {`for ${agent?.title?.trim() || "this agent"}, sent together as one review`}
        </Text>
        <View style={styles.grow} />
        <IdeTextButton label="Discard" icon={<Trash2 size={12} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => clearReviewComments(sessionId)} />
        <IdeTextButton label="Send review" tone="accent" icon={<Send size={12} strokeWidth={1.75} color={tokens.accent} />} onPress={sendReview} keyHint="⇧⌘↵" />
      </View>
      {open ? (
        <View style={styles.list}>
          {groupDraftByFile(comments).map((group) => (
            <View key={group.path} style={styles.group}>
              <Text numberOfLines={1} style={[styles.path, { color: tokens.ink2 }]}>
                {group.path}
              </Text>
              {group.comments.map((comment) => (
                <View key={comment.id} style={styles.item}>
                  <Keycap label={reviewRangeLabel(comment.side, comment.startLine, comment.endLine)} />
                  <Text numberOfLines={2} style={[styles.itemText, { color: tokens.ink }]} onPress={() => openChange(sessionId, comment.path)}>
                    {comment.text}
                  </Text>
                  <IdeIconButton icon={<Trash2 size={12} strokeWidth={1.75} color={tokens.ink3} />} label="Remove comment" onPress={() => removeReviewComment(sessionId, comment.id)} />
                </View>
              ))}
            </View>
          ))}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  composer: { gap: space.space4, padding: space.space12, borderTopWidth: StyleSheet.hairlineWidth },
  label: { fontFamily: sansFamilyFor(600), fontSize: 11.5 },
  quote: { fontFamily: monoFamily.regular, fontSize: 10.5, lineHeight: 15 },
  input: { minHeight: 56, maxHeight: 140, borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: space.space8, fontFamily: sansFamilyFor(400), fontSize: 12.5 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space4 },
  grow: { flex: 1 },
  bar: { borderTopWidth: StyleSheet.hairlineWidth },
  barRow: { minHeight: 36, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space8 },
  count: { fontFamily: sansFamilyFor(600), fontSize: 12 },
  sub: { flexShrink: 1, fontFamily: sansFamilyFor(400), fontSize: 11.5 },
  list: { maxHeight: 180, paddingHorizontal: space.space12, paddingBottom: space.space8, gap: space.space8, overflow: "scroll" as never },
  group: { gap: 2 },
  path: { fontFamily: monoFamily.medium, fontSize: 10.5 },
  item: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  itemText: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(400), fontSize: 12 },
});
