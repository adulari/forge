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
import { ArrowUp, Clock, FileText, Image as ImageIcon, RotateCcw, Square } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { Image, Platform, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { haptics } from "../../lib/haptics";
import { useUpload } from "../../lib/queries";
import { useSessionCtx } from "../../lib/sessionContext";
import { useTokens } from "../../theme/ThemeProvider";
import { gutter, radii, space, tapTarget } from "../../theme/tokens";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { type, webInputTextStyle } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import {
  formDataFromPicked,
  formDataFromWebFiles,
  imagesFromClipboardEvent,
  makeAttachmentId,
  type PickedFile,
  pickDocuments,
  pickImages,
  type SentAttachment,
} from "./attach";

const MAX_LINES = 6;
const LINE_HEIGHT = 22; // type.body line-height (DESIGN_SYSTEM §2)
const MIN_HEIGHT = LINE_HEIGHT;
const MAX_HEIGHT = LINE_HEIGHT * MAX_LINES;
const COMMAND_CHIPS = ["/plan", "/compact", "/model", "/mode", "/help"] as const;

export interface ComposerProps {
  sessionId: string;
  busy: boolean;
  /** true when the session WS is `open` — false swaps the send affordance to "queue". */
  online: boolean;
  onSend: (text: string, attachments: SentAttachment[]) => void;
  onInterrupt: () => void;
}

export function Composer({ sessionId, busy, online, onSend, onInterrupt }: ComposerProps) {
  const tokens = useTokens();
  const upload = useUpload();
  const { isCompact } = useBreakpoint();
  const insets = useSafeAreaInsets();
  // The parent `<Screen>` wraps ALL of its children (list, cards, composer) in one
  // paddingHorizontal'd View for the message-list gutter — that's wanted for MessageRow/
  // CardSlot, but it insets the composer's own bg2 panel from the true screen edges,
  // leaving Screen's bg1 showing through as a side gutter. Cancel it here with a matching
  // negative margin so the panel itself bleeds edge-to-edge; `styles.wrap`'s own
  // paddingHorizontal keeps the icons/input/send button reasonably inset from that edge.
  const screenGutter = isCompact ? gutter.compact : gutter.medium;

  // Draft (text + attachments) lives in SessionContext, not local state: the session shell
  // (`session/[id]/_layout.tsx`) mounts one SessionProvider per session and `router.replace`s
  // between Chat/Tasks/Agents/Review segments underneath it, which unmounts this component on
  // every tab switch — plain useState here would wipe a half-typed message. Keyed per session
  // (not per-component-instance) so it can't bleed across a session change either.
  const { draftText: text, setDraftText: setText, draftAttachments: attachments, setDraftAttachments: setAttachments, lastPrompt, setLastPrompt, composerFocusSignal } =
    useSessionCtx();
  const toast = useToast();
  const [height, setHeight] = useState(MIN_HEIGHT);
  const inputRef = useRef<TextInput>(null);
  const textRef = useRef(text);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  // Shell→Composer focus bridge: the session shell increments `composerFocusSignal` (e.g.
  // the ⌘E web shortcut) to request that this input take focus. A counter so repeated
  // requests always re-fire. Native `useHotkey` is a no-op, so this never fires there.
  useEffect(() => {
    if (composerFocusSignal === 0) return;
    inputRef.current?.focus();
  }, [composerFocusSignal]);

  const canSend = text.trim().length > 0 && !attachments.some((a) => a.state === "uploading");

  const commandHints = COMMAND_CHIPS.filter((cmd) => !text.startsWith("/") || cmd.startsWith(text.toLowerCase()));

  const commit = (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    // An attachment still in flight (or failed) must never be silently dropped from the send —
    // block it with a toast instead so the user notices and can wait/remove/retry.
    if (attachments.some((a) => a.state === "uploading")) {
      toast.show("still uploading — wait a moment", { tone: "warn" });
      return;
    }
    if (attachments.some((a) => a.state === "error")) {
      toast.show("an attachment failed — remove it or retry", { tone: "danger" });
      return;
    }
    // Only fully-uploaded attachments actually ride the daemon's next prompt (remote.rs
    // `RemoteInput::Attach`) — a failed upload never reached the session's pending queue, so it
    // must not be claimed on the sent bubble either.
    const sent: SentAttachment[] = attachments
      .filter((a) => a.state === "done")
      .map((a) => ({ id: a.id, name: a.name, image: a.image, uri: a.uri, path: a.path }));
    onSend(trimmed, sent);
    setLastPrompt(trimmed);
    haptics.sendPrompt();
    setText("");
    setAttachments([]);
    setHeight(MIN_HEIGHT);
  };
  // `commit` closes over `onSend` (and whatever connection state it reads), so it's a new
  // function every render. The web keydown listener below is only bound once per session —
  // route it through this ref so Enter always calls the CURRENT commit/onSend, not the one
  // captured when the listener was attached (otherwise a mount-while-disconnected session
  // would queue Enter-sent messages offline forever, even after the WS reconnects).
  const commitRef = useRef(commit);
  useEffect(() => {
    commitRef.current = commit;
  });

  const runUpload = async (picked: PickedFile, webFile?: File) => {
    const id = makeAttachmentId();
    setAttachments((prev) => [
      ...prev,
      { id, name: picked.name, image: picked.image, state: "uploading", uri: picked.uri },
    ]);
    try {
      // Web: a real `File` is required (RN's `{uri,name,type}` shorthand only means anything
      // to React Native's OWN native networking layer — a real browser `FormData` just
      // `String()`-coerces a plain object, uploading garbage). `picked.file` covers the
      // image/document pickers; `webFile` covers paste, which hands one over directly.
      const file = webFile ?? picked.file;
      const form = file ? formDataFromWebFiles([file]) : formDataFromPicked([picked]);
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
      // Enter (no Shift) already sends; ⌘/Ctrl+Enter (T5.1 alias) sends too, even in the
      // edge case both modifiers are held at once, so the desktop shortcut always works.
      if (e.key === "Enter" && (!e.shiftKey || e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commitRef.current(textRef.current);
      }
    };
    const onPasteEvt = (e: ClipboardEvent) => {
      const files = imagesFromClipboardEvent(e);
      for (const f of files) {
        const uri = URL.createObjectURL(f);
        void runUpload({ uri, name: f.name || "pasted-image.png", mimeType: f.type, image: true }, f);
      }
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
    <View
      style={[
        styles.wrap,
        {
          backgroundColor: tokens.bg2,
          borderTopColor: tokens.border,
          marginHorizontal: -screenGutter,
          // `Screen` omits "bottom" from its safe-area edges for this route (see index.tsx) so
          // this panel — not the screen's own bg1 — is what bleeds through the home-indicator
          // inset; otherwise a black strip would show below the grey composer.
          paddingBottom: space.space8 + insets.bottom,
        },
      ]}
    >
      {attachments.length > 0 ? (
        <View style={styles.chipsRow}>
          {attachments.map((a) => (
            <Chip
              key={a.id}
              icon={
                a.image && a.uri ? (
                  <Image source={{ uri: a.uri }} style={styles.chipThumb} />
                ) : (
                  <FileText size={14} strokeWidth={1.75} color={tokens.ink3} />
                )
              }
              label={a.state === "uploading" ? `${a.name} …` : a.state === "error" ? `${a.name} ⚠` : a.name}
              selected={a.state === "done"}
              onPress={() => removeAttachment(a.id)}
            />
          ))}
        </View>
      ) : null}

      <View style={styles.chipsRow}>
        {commandHints.map((cmd) => (
          <Chip key={cmd} label={cmd} onPress={() => commit(cmd)} testID={`chip-${cmd}`} />
        ))}
        {lastPrompt ? (
          <Chip
            label="resend last"
            icon={<RotateCcw size={14} strokeWidth={1.75} color={tokens.ink3} />}
            onPress={() => commit(lastPrompt)}
            testID="resend-last-prompt"
          />
        ) : null}
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
          style={[type.body, styles.input, webInputTextStyle, { color: tokens.ink, height }]}
          accessibilityLabel="message"
          testID="composer-input"
        />
        {busy ? (
          <IconButton
            icon={<Square size={16} strokeWidth={1.75} color={tokens.onAccent} fill={tokens.onAccent} />}
            onPress={() => {
              onInterrupt();
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
  chipThumb: { width: 20, height: 20, borderRadius: radii.radius4 },
  row: { flexDirection: "row", alignItems: "flex-end", gap: space.space4 },
  input: { flex: 1, paddingHorizontal: space.space8, paddingVertical: space.space8 },
  sendCircle: { borderRadius: radii.radiusPill, width: tapTarget, height: tapTarget },
  offlineHint: { paddingLeft: space.space4 },
});
