// DESIGN_SYSTEM.md §6 PromptComposer: multiline grow (max 6 lines), attach + send circle;
// send disabled when empty; offline state swaps the send circle to a queue glyph + "will
// send on reconnect" meta line. Web: Enter=send / Shift+Enter=newline + paste-image. Command
// Chips (`/plan` `/compact` `/models` `/mode` `/help`) insert the command and send it
// immediately (one-tap slash command execution, same idea as the command palette's "send as
// command"). Stop replaces the send circle while busy (`interrupt`).
//
// Mic input: DESIGN.md "Mobile/desktop (V3)" — press mic, the input row morphs into
// VoiceRecordingPill (lib/voice/ start/stop/cancel + POST /api/voice/transcribe), which
// appends the transcript to the draft and morphs back. Never auto-sends.
import { ArrowUp, Clock, FileText, Image as ImageIcon, Mic, RotateCcw, Sparkles, Square } from "lucide-react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withSequence, withTiming } from "react-native-reanimated";
import React, { useEffect, useRef, useState } from "react";
import { Image, Platform, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { haptics } from "../../lib/haptics";
import { clearDraft, getDraft, setDraft } from "../../lib/drafts";
import { isMacOS } from "../../lib/platform";
import { useUpload } from "../../lib/queries";
import { useSessionCtx } from "../../lib/sessionContext";
import { chordHold } from "../../lib/voice/chordHold";
import { voice } from "../../lib/voice/voice";
import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { gutter, radii, space, tapTarget } from "../../theme/tokens";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { type, webInputTextStyle } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { HeatEdge } from "../ds/HeatEdge";
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
import { GoalSheet } from "./GoalSheet";
import { VoiceRecordingPill } from "./VoiceRecordingPill";

const MAX_LINES = 6;
const LINE_HEIGHT = 22; // type.body line-height (DESIGN_SYSTEM §2)
const MIN_HEIGHT = 44;
const MAX_HEIGHT = LINE_HEIGHT * MAX_LINES;
const COMMAND_CHIPS = ["/goal", "/plan", "/compact", "/models", "/mode", "/help"] as const;

export interface ComposerProps {
  sessionId: string;
  busy: boolean;
  /** true when the session WS is `open` — false swaps the send affordance to "queue". */
  online: boolean;
  /** AI-suggested likely next user prompt (Snapshot.suggested_prompt) — surfaced as ghost text
   * + Tab-to-fill on a hardware keyboard, or a chip on touch. Never auto-sent. */
  suggestedPrompt?: string | null;
  onSend: (text: string, attachments: SentAttachment[]) => boolean;
  onInterrupt: () => void;
}

export function Composer({ sessionId, busy, online, suggestedPrompt, onSend, onInterrupt }: ComposerProps) {
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
  const {
    draftText: text,
    setDraftText: setText,
    draftAttachments: attachments,
    setDraftAttachments: setAttachments,
    lastPrompt,
    setLastPrompt,
    suppressedSuggestion,
    setSuppressedSuggestion,
    composerFocusSignal,
  } = useSessionCtx();
  const toast = useToast();
  const [recording, setRecording] = useState(false);
  const [goalVisible, setGoalVisible] = useState(false);
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [draftLoadedSession, setDraftLoadedSession] = useState<string | null>(null);
  const inputRef = useRef<TextInput>(null);
  const textRef = useRef(text);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  useEffect(() => {
    let cancelled = false;
    setDraftLoadedSession(null);
    setText("");
    void getDraft(sessionId).then((draft) => {
      if (cancelled) return;
      setText(draft ?? "");
      setDraftLoadedSession(sessionId);
    });
    return () => {
      cancelled = true;
    };
  }, [sessionId, setText]);

  useEffect(() => {
    if (draftLoadedSession !== sessionId) return;
    const timer = setTimeout(() => {
      if (text.trim().length === 0) {
        void clearDraft(sessionId);
      } else {
        void setDraft(sessionId, text);
      }
    }, 300);
    return () => clearTimeout(timer);
  }, [draftLoadedSession, sessionId, text]);

  // Shell→Composer focus bridge: the session shell increments `composerFocusSignal` (e.g.
  // the ⌘E web shortcut) to request that this input take focus. A counter so repeated
  // requests always re-fire. Native `useHotkey` is a no-op, so this never fires there.
  useEffect(() => {
    if (composerFocusSignal === 0) return;
    inputRef.current?.focus();
  }, [composerFocusSignal]);

  const canSend = text.trim().length > 0 && !attachments.some((a) => a.state === "uploading");
  const action = busy ? "stop" : online ? "send" : "queue";
  const reduced = useReducedMotion();
  const actionProgress = useSharedValue(action === "stop" ? 1 : 0);
  useEffect(() => {
    const target = action === "stop" ? 1 : 0;
    actionProgress.value = reduced ? target : withTiming(target, { duration: durations.fast, easing: easings.standard });
  }, [action, reduced, actionProgress]);
  const stopIconStyle = useAnimatedStyle(() => ({ opacity: actionProgress.value, transform: [{ scale: 0.8 + actionProgress.value * 0.2 }] }));
  const sendIconStyle = useAnimatedStyle(() => ({ opacity: 1 - actionProgress.value, transform: [{ scale: 1 - actionProgress.value * 0.2 }] }));

  const commandHints = COMMAND_CHIPS.filter((cmd) => !text.startsWith("/") || cmd.startsWith(text.toLowerCase()));

  // `suggestedPrompt` keeps echoing the STALE pre-send value for a beat after a send clears the
  // draft (the daemon hasn't refreshed it yet) — `suppressedSuggestion` (SessionContext, set in
  // `commit` below) masks that exact stale string until the server actually produces a new one.
  const activeSuggestion =
    suggestedPrompt && suggestedPrompt !== suppressedSuggestion ? suggestedPrompt : null;
  const suggestionActive =
    activeSuggestion != null &&
    text.trim().length === 0 &&
    !recording &&
    attachments.length === 0 &&
    !busy;
  const showGhost = Platform.OS === "web" && suggestionActive;
  const showSuggestionChip = Platform.OS !== "web" && suggestionActive;

  // Cross-fade the suggestion in on first appearance (fade + slight rise, mirrors Forgeline),
  // quick-crossfade when one suggestion replaces another while visible, fade out otherwise —
  // never a layout jump, since both the ghost overlay and the chip's own wrapper keep their box.
  const suggestionOpacity = useSharedValue(0);
  const suggestionTranslateY = useSharedValue(6);
  const prevSuggestionRef = useRef<string | null>(null);
  useEffect(() => {
    const prev = prevSuggestionRef.current;
    prevSuggestionRef.current = activeSuggestion;
    if (reduced) {
      suggestionOpacity.value = activeSuggestion ? 1 : 0;
      suggestionTranslateY.value = 0;
      return;
    }
    if (activeSuggestion == null) {
      suggestionOpacity.value = withTiming(0, { duration: durations.fast, easing: easings.standard });
      return;
    }
    if (prev == null) {
      suggestionOpacity.value = 0;
      suggestionTranslateY.value = 6;
      suggestionOpacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
      suggestionTranslateY.value = withTiming(0, { duration: durations.base, easing: easings.standard });
    } else if (prev !== activeSuggestion) {
      suggestionOpacity.value = withSequence(
        withTiming(0, { duration: durations.instant, easing: easings.standard }),
        withTiming(1, { duration: durations.instant, easing: easings.standard }),
      );
    } else {
      suggestionOpacity.value = 1;
      suggestionTranslateY.value = 0;
    }
  }, [activeSuggestion, reduced, suggestionOpacity, suggestionTranslateY]);
  const suggestionFadeStyle = useAnimatedStyle(() => ({ opacity: suggestionOpacity.value }));
  const suggestionChipStyle = useAnimatedStyle(() => ({
    opacity: suggestionOpacity.value,
    transform: [{ translateY: suggestionTranslateY.value }],
  }));

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
    if (!onSend(trimmed, sent)) return;
    setLastPrompt(trimmed);
    // Whatever suggestion is live right now belongs to the turn that's ending, not the one this
    // send just started — mask it so it can't flash back in once the draft clears below.
    if (suggestedPrompt) setSuppressedSuggestion(suggestedPrompt);
    haptics.sendPrompt();
    setText("");
    void clearDraft(sessionId);
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

  // Reads `textRef` (not the closed-over `text`) for the same reason `commitRef` does below —
  // this callback is handed to VoiceRecordingPill once and must see the CURRENT draft when the
  // async transcription resolves, not whatever draft existed when recording started.
  const appendTranscript = (transcript: string) => {
    const trimmed = transcript.trim();
    if (!trimmed) return;
    const current = textRef.current;
    setText(current.trim().length > 0 ? `${current} ${trimmed}` : trimmed);
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

  // Web ghost text: plain Tab accepts the live suggestion into the (editable) draft — never
  // auto-sends. Document-level (not the input node) so it also fires when nothing is focused
  // yet, but yields to Shift+Tab (reverse focus) and to any OTHER input/textarea already
  // holding focus — the composer's own input is exempted since that's the natural place to
  // invoke this from.
  useEffect(() => {
    if (Platform.OS !== "web" || !showGhost || !activeSuggestion) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || e.shiftKey || e.repeat || recording) return;
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        target !== (inputRef.current as unknown as HTMLElement | null) &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      const suggestion = activeSuggestion;
      setText(suggestion);
      const node = inputRef.current as unknown as HTMLTextAreaElement | null;
      // Cursor must land at the end, editable — the value swap alone doesn't reliably move it
      // on every browser, so place it explicitly once the DOM has the new value.
      requestAnimationFrame(() => {
        node?.focus();
        node?.setSelectionRange?.(suggestion.length, suggestion.length);
      });
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [showGhost, activeSuggestion, recording, setText]);

  // Web/desktop: Ctrl/Cmd+Shift+V starts voice recording from anywhere in the window, not just
  // when the composer input is focused — mirrors the mic button's own onPress. Document-level
  // (not the input node) so it fires regardless of focus; the stop side (tap-toggle, Enter,
  // Escape, AND push-to-talk release — the chord's keyup lands after this row has swapped to
  // the pill) lives in VoiceRecordingPill, which owns the recording state machine once mounted,
  // so this listener only ever flips idle -> recording and the two can't race each other.
  // `chordHold.startedAt` stamps the hold start so the pill can tell a tap (<400ms, stay in
  // toggle mode) from a hold (push-to-talk: stop + transcribe on release).
  useEffect(() => {
    if (Platform.OS !== "web" || !voice.isSupported()) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || recording) return;
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey || e.key.toLowerCase() !== "v") return;
      if (attachments.some((a) => a.state === "uploading")) return; // mirrors the mic button's own disabled state
      const target = e.target;
      // A text input/textarea elsewhere on the page keeps typing priority — except the
      // composer's own input, which is the natural place to invoke this shortcut from.
      if (
        target instanceof HTMLElement &&
        target !== (inputRef.current as unknown as HTMLElement | null) &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      chordHold.startedAt = Date.now();
      setRecording(true);
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [recording, attachments]);

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
      <HeatEdge active={busy} />
      {!recording && attachments.length > 0 ? (
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

      {!recording ? (
        <View style={styles.chipsRow}>
          {text.length === 0 ? <Chip label="goal" icon={<Sparkles size={14} strokeWidth={1.75} color={tokens.ink3} />} onPress={() => setGoalVisible(true)} testID="chip-goal" /> : null}
          {text.length === 0 || text.startsWith("/") ? commandHints.map((cmd) => (
            <Chip key={cmd} label={cmd} onPress={() => commit(cmd)} testID={`chip-${cmd}`} />
          )) : null}
          {lastPrompt ? (
            <Chip
              label="resend last"
              icon={<RotateCcw size={14} strokeWidth={1.75} color={tokens.ink3} />}
              onPress={() => commit(lastPrompt)}
              testID="resend-last-prompt"
            />
          ) : null}
          {showSuggestionChip && activeSuggestion ? (
            <Animated.View style={suggestionChipStyle}>
              <Chip
                label={activeSuggestion}
                icon={<Sparkles size={14} strokeWidth={1.75} color={tokens.accent} />}
                onPress={() => setText(activeSuggestion)}
                testID="suggested-prompt-chip"
              />
            </Animated.View>
          ) : null}
        </View>
      ) : null}

      {recording ? (
        <VoiceRecordingPill
          onAppend={(transcript) => {
            appendTranscript(transcript);
            setRecording(false);
          }}
          onClose={() => setRecording(false)}
        />
      ) : (
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
          <View style={styles.inputWrap}>
            <TextInput
              ref={inputRef}
              value={text}
              onChangeText={setText}
              returnKeyType="send"
              autoCapitalize="none"
              autoCorrect={false}
              spellCheck={false}
              onContentSizeChange={(e) =>
                setHeight(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, e.nativeEvent.contentSize.height)))
              }
              multiline
              textAlignVertical="top"
              // Hidden while the ghost is showing — it renders the same "empty state" copy
              // itself, and both at once would double up.
              placeholder={showGhost ? undefined : "message…"}
              placeholderTextColor={tokens.ink3}
              style={[type.body, styles.input, webInputTextStyle, { color: tokens.ink, height }]}
              accessibilityLabel="message"
              testID="composer-input"
            />
            {showGhost && activeSuggestion ? (
              // True ghost text: same font/size/padding/lineHeight as the TextInput above,
              // absolutely positioned over it. Only ever rendered while `text` is empty (see
              // `suggestionActive`), so it never has to track the caret mid-string.
              <Animated.View style={[styles.ghostOverlay, suggestionFadeStyle]} pointerEvents="none">
                <Text
                  style={[type.body, webInputTextStyle, { color: tokens.ink3 }]}
                  numberOfLines={1}
                  testID="suggested-prompt-ghost"
                >
                  {activeSuggestion}
                  <Text style={[type.meta, { color: tokens.ink4 }]}>  ⇥ tab</Text>
                </Text>
              </Animated.View>
            ) : null}
          </View>
          {voice.isSupported() ? (
            <IconButton
              icon={<Mic size={20} strokeWidth={1.75} color={tokens.ink2} />}
              onPress={() => setRecording(true)}
              disabled={attachments.some((a) => a.state === "uploading")}
              accessibilityLabel={
                Platform.OS === "web"
                  ? `record voice message — tap to toggle, hold to talk (${isMacOS ? "⌘" : "Ctrl"}+Shift+V)`
                  : "record voice message"
              }
              testID="composer-mic"
            />
          ) : null}
          <IconButton
            icon={<View style={styles.actionIcon}><Animated.View style={sendIconStyle}>{online ? <ArrowUp size={20} strokeWidth={2} color={canSend ? tokens.onAccent : tokens.ink4} /> : <Clock size={18} strokeWidth={1.75} color={canSend ? tokens.onAccent : tokens.ink4} />}</Animated.View><Animated.View style={[styles.actionLayer, stopIconStyle]}><Square size={16} strokeWidth={1.75} color={tokens.onAccent} fill={tokens.onAccent} /></Animated.View></View>}
            onPress={busy ? onInterrupt : () => commit(text)}
            disabled={!busy && !canSend}
            accessibilityLabel={busy ? "stop" : online ? "send" : "queue — will send on reconnect"}
            style={[styles.sendCircle, { backgroundColor: busy ? tokens.danger : canSend ? tokens.accent : tokens.bg3 }]}
          />
        </View>
      )}
      {!online && !recording ? (
        <Text style={[type.meta, styles.offlineHint, { color: tokens.ink3 }]}>will send on reconnect</Text>
      ) : null}
      <GoalSheet visible={goalVisible} onClose={() => setGoalVisible(false)} onSubmit={commit} />
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    position: "relative",
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: space.space12,
    paddingTop: space.space8,
    paddingBottom: space.space8,
    gap: space.space8,
  },
  chipsRow: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  chipThumb: { width: 20, height: 20, borderRadius: radii.radius4 },
  row: { flexDirection: "row", alignItems: "flex-end", gap: space.space4 },
  input: { flex: 1, paddingHorizontal: space.space8, paddingVertical: space.space8, textAlignVertical: "top" },
  inputWrap: { flex: 1, position: "relative", overflow: "visible" },
  // Mirrors `input`'s own padding exactly so the ghost text lines up with where a typed
  // caret would sit — only ever shown while the TextInput is empty (see `suggestionActive`).
  ghostOverlay: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    paddingHorizontal: space.space8,
    paddingVertical: space.space8,
  },
  sendCircle: { borderRadius: radii.radiusPill, width: tapTarget, height: tapTarget },
  actionIcon: { width: 20, height: 20, alignItems: "center", justifyContent: "center" },
  actionLayer: { position: "absolute" },
  offlineHint: { paddingLeft: space.space4 },
});
