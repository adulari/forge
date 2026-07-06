// DESIGN_SYSTEM.md §6 PromptComposer: multiline grow (max 6 lines), attach + send circle;
// send disabled when empty; offline state swaps the send circle to a queue glyph + "will
// send on reconnect" meta line. Web: Enter=send / Shift+Enter=newline + paste-image. Command
// Chips (`/plan` `/compact` `/models` `/mode` `/help`) insert the command and send it
// immediately (one-tap slash command execution, same idea as the command palette's "send as
// command"). Stop replaces the send circle while busy (`interrupt`).
//
// Mic input is intentionally NOT built here: FEATURES.md §5 flags it as a separate `lib/voice/`
// seam (web Speech API now, native mic deferred) that BUILD_ORDER's T3.2 bullet does not list —
// a non-functional mic button would be worse than none.
import { ArrowUp, Clock, FileText, Image as ImageIcon, Square } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { Platform, StyleSheet, Text, TextInput, View } from "react-native";

import { haptics } from "../../lib/haptics";
import { useUpload } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { IconButton } from "../ds/IconButton";
import {
  type Attachment,
  formDataFromPicked,
  formDataFromWebFiles,
  imagesFromClipboardEvent,
  makeAttachmentId,
  type PickedFile,
  pickDocuments,
  pickImages,
} from "./attach";

const MAX_LINES = 6;
const LINE_HEIGHT = 22; // type.body line-height (DESIGN_SYSTEM §2)
const MIN_HEIGHT = LINE_HEIGHT;
const MAX_HEIGHT = LINE_HEIGHT * MAX_LINES;
const COMMAND_CHIPS = ["/plan", "/compact", "/models", "/mode", "/help"] as const;

export interface ComposerProps {
  sessionId: string;
  busy: boolean;
  /** true when the session WS is `open` — false swaps the send affordance to "queue". */
  online: boolean;
  onSend: (text: string) => void;
  onInterrupt: () => void;
}

export function Composer({ sessionId, busy, online, onSend, onInterrupt }: ComposerProps) {
  const tokens = useTokens();
  const upload = useUpload();

  const [text, setText] = useState("");
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const inputRef = useRef<TextInput>(null);
  const textRef = useRef(text);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  const canSend = text.trim().length > 0 && !attachments.some((a) => a.state === "uploading");

  const commit = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    onSend(trimmed);
    haptics.sendPrompt();
    setText("");
    setAttachments([]);
    setHeight(MIN_HEIGHT);
  };

  const runUpload = async (picked: PickedFile, webFile?: File) => {
    const id = makeAttachmentId();
    setAttachments((prev) => [...prev, { id, name: picked.name, image: picked.image, state: "uploading" }]);
    try {
      const form = webFile ? formDataFromWebFiles([webFile]) : formDataFromPicked([picked]);
      const res = await upload.mutateAsync({ sessionId, form });
      const uploaded = res.files[0];
      setAttachments((prev) =>
        prev.map((a) => (a.id === id ? { ...a, state: "done", path: uploaded?.path } : a)),
      );
    } catch {
      setAttachments((prev) => prev.map((a) => (a.id === id ? { ...a, state: "error" } : a)));
    }
  };

  const onAttachImage = async () => {
    const picked = await pickImages();
    for (const p of picked) void runUpload(p);
  };

  const onAttachDocument = async () => {
    const picked = await pickDocuments();
    for (const p of picked) void runUpload(p);
  };

  const removeAttachment = (id: string) => {
    setAttachments((prev) => prev.filter((a) => a.id !== id));
  };

  // Web: Enter=send / Shift+Enter=newline + paste-image. Bound directly to the underlying DOM
  // node (RN-Web's TextInput ref IS that node) since RN's onKeyPress doesn't expose `shiftKey`.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = inputRef.current as unknown as HTMLTextAreaElement | null;
    if (!node) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        commit(textRef.current);
      }
    };
    const onPasteEvt = (e: ClipboardEvent) => {
      const files = imagesFromClipboardEvent(e);
      for (const f of files) void runUpload({ uri: "", name: f.name || "pasted-image.png", mimeType: f.type, image: true }, f);
    };

    node.addEventListener("keydown", onKeyDown);
    node.addEventListener("paste", onPasteEvt);
    return () => {
      node.removeEventListener("keydown", onKeyDown);
      node.removeEventListener("paste", onPasteEvt);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  return (
    <View style={[styles.wrap, { backgroundColor: tokens.bg2, borderTopColor: tokens.border }]}>
      {attachments.length > 0 ? (
        <View style={styles.chipsRow}>
          {attachments.map((a) => (
            <Chip
              key={a.id}
              label={a.state === "uploading" ? `${a.name} …` : a.state === "error" ? `${a.name} ⚠` : a.name}
              selected={a.state === "done"}
              onPress={() => removeAttachment(a.id)}
            />
          ))}
        </View>
      ) : null}

      <View style={styles.chipsRow}>
        {COMMAND_CHIPS.map((cmd) => (
          <Chip key={cmd} label={cmd} onPress={() => commit(cmd)} testID={`chip-${cmd}`} />
        ))}
      </View>

      <View style={styles.row}>
        <IconButton
          icon={<ImageIcon size={20} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={onAttachImage}
          accessibilityLabel="attach photo"
        />
        <IconButton
          icon={<FileText size={20} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={onAttachDocument}
          accessibilityLabel="attach file"
        />
        <TextInput
          ref={inputRef}
          value={text}
          onChangeText={setText}
          onContentSizeChange={(e) =>
            setHeight(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, e.nativeEvent.contentSize.height)))
          }
          multiline
          placeholder="message…"
          placeholderTextColor={tokens.ink3}
          style={[type.body, styles.input, { color: tokens.ink, height }]}
          accessibilityLabel="message"
          testID="composer-input"
        />
        {busy ? (
          <IconButton
            icon={<Square size={16} strokeWidth={1.75} color={tokens.onAccent} fill={tokens.onAccent} />}
            onPress={() => {
              onInterrupt();
              haptics.deny();
            }}
            accessibilityLabel="stop"
            style={[styles.sendCircle, { backgroundColor: tokens.danger }]}
          />
        ) : (
          <IconButton
            icon={
              online ? (
                <ArrowUp size={20} strokeWidth={2} color={canSend ? tokens.onAccent : tokens.ink4} />
              ) : (
                <Clock size={18} strokeWidth={1.75} color={canSend ? tokens.onAccent : tokens.ink4} />
              )
            }
            onPress={() => commit(text)}
            disabled={!canSend}
            accessibilityLabel={online ? "send" : "queue — will send on reconnect"}
            style={[styles.sendCircle, { backgroundColor: canSend ? tokens.accent : tokens.bg3 }]}
          />
        )}
      </View>
      {!online ? (
        <Text style={[type.meta, styles.offlineHint, { color: tokens.ink3 }]}>will send on reconnect</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: space.space12,
    paddingTop: space.space8,
    paddingBottom: space.space8,
    gap: space.space8,
  },
  chipsRow: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  row: { flexDirection: "row", alignItems: "flex-end", gap: space.space4 },
  input: { flex: 1, paddingHorizontal: space.space8, paddingVertical: space.space8 },
  sendCircle: { borderRadius: radii.radiusPill, width: tapTarget, height: tapTarget },
  offlineHint: { paddingLeft: space.space4 },
});
