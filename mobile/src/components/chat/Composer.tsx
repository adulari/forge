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
//
// Model/effort chips (D Main / M Session Chat control row) are real pickers, driven entirely by
// the mid-session switch path that already exists — no new wire message was invented:
//   model  → `{kind:"prompt", text:"/model"}`, the daemon's own command (commands.rs
//            `PinModel(None)`). The TUI opens its model picker, that overlay is mirrored back on
//            `Snapshot.overlay`, and OverlayHost (mounted by the session shell) renders it. Same
//            call TelemetrySheet's "Model" row already makes.
//   effort → the existing EffortPicker, mounted headless (`showTrigger={false}`), which commits
//            `/effort <level>` (or bare `/effort` to reset) over the same prompt path.
import { ArrowUp, ChevronDown, Clock, FileText, Gauge, Image as ImageIcon, ListEnd, MessageSquare, Mic, MousePointer2, RotateCcw, ShieldHalf, Sparkles, Square, Upload, Zap } from "lucide-react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withSequence, withTiming } from "react-native-reanimated";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type ColorValue, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { onModelPickerRequest } from "../../lib/alertTargets";
import { haptics } from "../../lib/haptics";
import { BUILTIN_COMMANDS, isKnownCommand, useSkillCommands } from "../../lib/commands";
import { useAppShortcut } from "../../lib/shortcuts";
import { mergeCommandSources } from "../../lib/commandSources";
import { EMPTY_EDITS, type EditStack, recordEdit, redoEdit, undoEdit } from "../../lib/composerUndo";
import { clearDraft, clearDraftAttachments, getDraft, getDraftAttachments, setDraft, setDraftAttachments } from "../../lib/drafts";
import { effectiveMode, type FollowUpMode, FOLLOW_UP_KEY } from "../../lib/followUp";
import { useFollowUpSetting } from "../../lib/useFollowUpSetting";
import { collapsePrompts, RECALL_IDLE, type RecallState, recallStep } from "../../lib/promptRecall";
import {
  clearReviewComments,
  formatReviewCommentsPrompt,
  removeReviewComment,
  reviewRangeLabel,
  useReviewComments,
} from "../../lib/reviewComments";
import {
  clearVisualAnnotations,
  formatVisualAnnotationsPrompt,
  removeVisualAnnotation,
  useVisualAnnotations,
  visualAnnotationLabel,
} from "../../lib/visualAnnotations";
import { isMacOS } from "../../lib/platform";
import { recordComposerImeCommit, recordComposerInput } from "../../lib/performance";
import { useCachedSessions, useUpdateConfig, useUpload, useWorkspaceSearch } from "../../lib/queries";
import type { WorkspaceSearchResult } from "../../lib/api";
import { mentionOptions, type MentionOption, removeMention } from "../../lib/mentionsModel";
import { useSessionDraft, useSessionStable } from "../../lib/sessionContext";
import { chordHold } from "../../lib/voice/chordHold";
import { voice } from "../../lib/voice/voice";
import { durations, easings } from "../../theme/motion";
import { useTheme } from "../../theme/ThemeProvider";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { depthDark, depthLight, radii, shadowStyle, space, tapTarget } from "../../theme/tokens";
import { type, webInputTextStyle } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { IconButton } from "../ds/IconButton";
import { useToast } from "../ds/ToastHost";
import { AttachmentTray } from "./AttachmentTray";
import { MentionChips } from "./MentionChips";
import { MentionMenuRow } from "./MentionMenuRow";
import { MiniPressable } from "./MiniPressable";
import {
  filesFromDragEvent,
  formDataFromPicked,
  formDataFromWebFiles,
  imagesFromClipboardEvent,
  makeAttachmentId,
  type PickedFile,
  pickedFromDroppedFile,
  pickDocuments,
  pickImages,
  type SentAttachment,
} from "./attach";
import {
  clampComposerHeight,
  composerInputVerticalPadding,
  composerScrollEnabled,
  composerUsesNativeMirror,
  textFitsOneLine,
  COMPOSER_LINE_HEIGHT as LINE_HEIGHT,
  COMPOSER_MAX_HEIGHT as MAX_HEIGHT,
  COMPOSER_MIN_HEIGHT as MIN_HEIGHT,
  nativeComposerMirrorText,
} from "./composerSizing";
import { EffortPicker } from "../session/EffortPicker";
import {
  replaceWorkspaceMention,
  workspaceMentionAtEnd,
} from "../workspace/workspaceModel";
import { GoalSheet } from "./GoalSheet";
import { VoiceRecordingPill } from "./VoiceRecordingPill";
import { ProviderGlyph } from "../ds/ProviderGlyph";
import { composerPlaceholder, modelChipLabel } from "./modelChipLabel";

const ICON_ONLY_CHIPS_BELOW = 420;
const RECALL_SENT_MAX = 50;

interface NativeComposerMirrorProps {
  color: ColorValue;
  text: string;
}

function NativeComposerMirror({ color, text }: NativeComposerMirrorProps) {
  return (
    <Text
      pointerEvents="none"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[type.body, styles.inputTextMetrics, styles.nativeMirror, { color }]}
      testID="composer-input-mirror"
    >
      {nativeComposerMirrorText(text)}
    </Text>
  );
}

export interface ComposerProps {
  sessionId: string;
  busy: boolean;
  /** true when the session WS is `open` — false swaps the send affordance to "queue". */
  online: boolean;
  /** AI-suggested likely next user prompt (Snapshot.suggested_prompt) — surfaced as ghost text
   * + Tab-to-fill on a hardware keyboard, or a chip on touch. Never auto-sent. */
  suggestedPrompt?: string | null;
  /** Snapshot.model / Snapshot.effort, passed down by the caller — Composer itself only
   * reads the stable session context (see `useSessionStable` below) so it can be wrapped in
   * `React.memo` without re-rendering on every WS snapshot frame. */
  model?: string | null;
  /** From the host: `false` means `model` is only the mesh's latest pick for an Automatic
   * session; `undefined` (older host) is unknown. */
  modelPinned?: boolean;
  effort?: string | null;
  /** Snapshot.temper (the operating mode label); its chip opens the daemon's own `/mode` picker. */
  temper?: string | null;
  /** `mode` only matters mid-turn: "queue" waits behind the turn, "steer" also offers the
   * prompt to it at its next step. */
  onSend: (text: string, attachments: SentAttachment[], mode?: FollowUpMode) => boolean;
  onInterrupt: () => void;
  /** The session's earlier user prompts, oldest first, for ArrowUp recall. */
  recallPrompts?: readonly string[];
}

function ComposerImpl({ sessionId, busy, online, suggestedPrompt, model, modelPinned, effort, temper, onSend, onInterrupt, recallPrompts }: ComposerProps) {
  const { scheme, tokens } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;
  const imeProps = { onCompositionEnd: recordComposerImeCommit } as unknown as React.ComponentProps<typeof TextInput>;
  const usesNativeMirror = composerUsesNativeMirror(Platform.OS);
  const upload = useUpload();
  const insets = useSafeAreaInsets();
  const [commandFocusSignal, setCommandFocusSignal] = useState(0);

  // Draft (text + attachments) lives in SessionContext, not local state: the session shell
  // (`session/[id]/_layout.tsx`) mounts one SessionProvider per session and `router.replace`s
  // between Chat/Tasks/Agents/Review segments underneath it, which unmounts this component on
  // every tab switch — plain useState here would wipe a half-typed message. Keyed per session
  // (not per-component-instance) so it can't bleed across a session change either.
  const { draftText: text, draftAttachments: attachments } = useSessionDraft();
  const {
    setDraftText: setText,
    setDraftAttachments: setAttachments,
    lastPrompt,
    setLastPrompt,
    suppressedSuggestion,
    setSuppressedSuggestion,
    composerFocusSignal,
    // The session socket's own sender — the model/effort chips are pickers over the existing
    // slash-command path (see file header), which is a `RemoteInput`, not an `onSend` prompt.
    send,
    baseUrl,
  } = useSessionStable();
  const followUp = useFollowUpSetting();
  const updateConfig = useUpdateConfig();
  const { width: windowWidth } = useWindowDimensions();
  const iconOnlyChips = windowWidth < ICON_ONLY_CHIPS_BELOW;
  const reviewComments = useReviewComments(sessionId);
  const visualAnnotations = useVisualAnnotations(sessionId);
  const toast = useToast();
  const [recording, setRecording] = useState(false);
  const [goalVisible, setGoalVisible] = useState(false);
  const [effortVisible, setEffortVisible] = useState(false);
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [nativeText, setNativeText] = useState(text);
  const [focused, setFocused] = useState(false);
  // The command chips show while the composer is in use, not as a permanent row of big pills
  // taking a phone's screen. A blur hides them a beat later, so a tap on a chip (which blurs
  // the input first on web) still lands.
  const [chipsOpen, setChipsOpen] = useState(false);
  const { isCompact } = useBreakpoint();
  const chipsTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (chipsTimer.current) clearTimeout(chipsTimer.current); }, []);
  const onInputFocus = useCallback(() => {
    if (chipsTimer.current) clearTimeout(chipsTimer.current);
    setFocused(true);
    setChipsOpen(true);
  }, []);
  const onInputBlur = useCallback(() => {
    setFocused(false);
    if (chipsTimer.current) clearTimeout(chipsTimer.current);
    chipsTimer.current = setTimeout(() => setChipsOpen(false), 300);
  }, []);
  const [draftLoadedSession, setDraftLoadedSession] = useState<string | null>(null);
  const inputRef = useRef<TextInput>(null);
  const wrapRef = useRef<View>(null);
  const [dragging, setDragging] = useState(false);
  const textRef = useRef(text);
  const editsRef = useRef<EditStack>(EMPTY_EDITS);
  const recallRef = useRef<{ state: RecallState; shown: string | null }>({ state: RECALL_IDLE, shown: null });
  const sentRef = useRef<string[]>([]);
  const recallEntriesRef = useRef<string[]>([]);
  useEffect(() => {
    recallEntriesRef.current = collapsePrompts([...(recallPrompts ?? []), ...sentRef.current]);
  }, [recallPrompts, text]);
  const measuredForText = useRef<string | null>(null);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  useEffect(() => {
    if (Platform.OS !== "web" && text !== nativeText) setNativeText(text);
  }, [nativeText, text]);

  // Web autosize: RNW's onContentSizeChange goes quiet once an explicit height style is set,
  // so measure the textarea's natural scrollHeight directly on every text change (collapse →
  // read → restore, the classic autosize pattern). Grows to MAX_LINES, then scrolls.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = inputRef.current as unknown as HTMLTextAreaElement | null;
    // (`scrollHeight` is never read in this guard: reading it forces a synchronous layout.)
    if (!node || !node.style) return;
    // Effects re-run when the segment is revealed again (see SegmentKeepAlive); the height was
    // already measured for this exact text, so there is nothing to do.
    if (measuredForText.current === text) return;
    measuredForText.current = text;
    if (textFitsOneLine(node, text)) {
      setHeight(MIN_HEIGHT);
      return;
    }
    const prev = node.style.height;
    node.style.height = "0px";
    const content = node.scrollHeight;
    node.style.height = prev;
    setHeight(clampComposerHeight(content));
  }, [text]);

  // The session whose stored draft has been loaded into this composer. When a kept-alive session
  // is shown again (its effects re-run) the in-memory draft is newer than storage's 300 ms
  // debounce, so reloading would clobber what was typed before switching away.
  const draftLoadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (draftLoadedFor.current === sessionId) return;
    let cancelled = false;
    setDraftLoadedSession(null);
    setText("");
    editsRef.current = EMPTY_EDITS;
    recallRef.current = { state: RECALL_IDLE, shown: null };
    sentRef.current = [];
    void Promise.all([getDraft(sessionId), getDraftAttachments(sessionId)]).then(([draft, storedAttachments]) => {
      if (cancelled) return;
      setText(draft ?? "");
      if (storedAttachments.length > 0) setAttachments((prev) => (prev.length > 0 ? prev : storedAttachments));
      draftLoadedFor.current = sessionId;
      setDraftLoadedSession(sessionId);
    });
    return () => {
      cancelled = true;
    };
  }, [sessionId, setText, setAttachments]);

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

  useEffect(() => {
    if (draftLoadedSession !== sessionId) return;
    const timer = setTimeout(() => void setDraftAttachments(sessionId, attachments), 300);
    return () => clearTimeout(timer);
  }, [draftLoadedSession, sessionId, attachments]);

  // Shell→Composer focus bridge: the session shell increments `composerFocusSignal` (e.g.
  // the ⌘E web shortcut) to request that this input take focus. A counter so repeated
  // requests always re-fire. Native `useHotkey` is a no-op, so this never fires there.
  useEffect(() => {
    if (composerFocusSignal === 0) return;
    inputRef.current?.focus();
  }, [composerFocusSignal]);

  useEffect(() => {
    if (commandFocusSignal === 0) return;
    inputRef.current?.focus();
  }, [commandFocusSignal]);

  // An edit the composer makes itself (voice transcript, @-mention, accepted suggestion) is
  // recorded so Ctrl/Cmd+Z can take it back — the browser's own undo forgets a value set from code.
  const applyProgrammatic = useCallback(
    (next: string) => {
      editsRef.current = recordEdit(editsRef.current, textRef.current, next);
      recallRef.current = { state: RECALL_IDLE, shown: null };
      if (Platform.OS !== "web") setNativeText(next);
      setText(next);
    },
    [setText],
  );
  const focusEnd = useCallback((value: string) => {
    requestAnimationFrame(() => {
      const node = inputRef.current as unknown as HTMLTextAreaElement | null;
      node?.focus();
      node?.setSelectionRange?.(value.length, value.length);
    });
  }, []);

  const canSend =
    (text.trim().length > 0 || reviewComments.length > 0 || visualAnnotations.length > 0) &&
    !attachments.some((a) => a.state === "uploading");
  // While a turn runs the circle is Stop until there is something to say; typed text turns it
  // into "steer": the prompt is queued and handed to the model at the turn's next step (the same
  // behaviour as typing during a turn in the CLI — the daemon's queue + steer inbox). A separate
  // small Stop stays reachable next to it so steering never hides the interrupt.
  const action = busy ? (canSend ? "steer" : "stop") : online ? "send" : "queue";
  const reduced = useReducedMotion();
  const actionProgress = useSharedValue(action === "stop" ? 1 : 0);
  useEffect(() => {
    const target = action === "stop" ? 1 : 0;
    actionProgress.value = reduced ? target : withTiming(target, { duration: durations.fast, easing: easings.standard });
  }, [action, reduced, actionProgress]);
  const stopIconStyle = useAnimatedStyle(() => ({ opacity: actionProgress.value, transform: [{ scale: 0.8 + actionProgress.value * 0.2 }] }));
  const sendIconStyle = useAnimatedStyle(() => ({ opacity: 1 - actionProgress.value, transform: [{ scale: 1 - actionProgress.value * 0.2 }] }));

  const skillCommands = useSkillCommands();
  const allCommands = mergeCommandSources(BUILTIN_COMMANDS, skillCommands).map((command) => command.name);
  const commandHints = allCommands.filter((cmd) => !text.startsWith("/") || cmd.startsWith(text.toLowerCase()));
  const leadingCommand = text.match(/^\/(\S*)/)?.[0].toLowerCase();
  const recognizedCommand = leadingCommand != null && isKnownCommand(leadingCommand, skillCommands.map((s) => s.name));
  const workspaceMention = workspaceMentionAtEnd(text);
  const workspaceMentionQuery = workspaceMention?.query ?? "";
  const workspaceSearch = useWorkspaceSearch(
    sessionId,
    workspaceMentionQuery,
    "files",
    8,
  );
  const cachedSessions = useCachedSessions();
  const contextMentions = useMemo(
    () => (workspaceMention ? mentionOptions(workspaceMentionQuery, cachedSessions, sessionId) : []),
    [workspaceMention, workspaceMentionQuery, cachedSessions, sessionId],
  );
  const workspaceMentionResults = useMemo<(WorkspaceSearchResult | MentionOption)[]>(
    () => [...contextMentions, ...(workspaceSearch.data?.results ?? [])],
    [contextMentions, workspaceSearch.data],
  );
  const [workspaceMentionSelection, setWorkspaceMentionSelection] = useState(0);
  const workspaceMentionRef = useRef(workspaceMention);
  const workspaceMentionResultsRef = useRef(workspaceMentionResults);
  const workspaceMentionSelectionRef = useRef(workspaceMentionSelection);

  useEffect(() => {
    setWorkspaceMentionSelection(0);
  }, [workspaceMentionQuery, workspaceSearch.data]);

  const insertWorkspaceMention = useCallback((path: string) => {
    const mention = workspaceMentionRef.current;
    if (!mention) return;
    const next = replaceWorkspaceMention(textRef.current, mention, path);
    applyProgrammatic(next);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [applyProgrammatic]);
  const removeMentionChip = useCallback(
    (token: string) => applyProgrammatic(removeMention(textRef.current, token)),
    [applyProgrammatic],
  );
  const insertWorkspaceMentionRef = useRef(insertWorkspaceMention);
  useEffect(() => {
    workspaceMentionRef.current = workspaceMention;
    workspaceMentionResultsRef.current = workspaceMentionResults;
    workspaceMentionSelectionRef.current = workspaceMentionSelection;
    insertWorkspaceMentionRef.current = insertWorkspaceMention;
  }, [
    insertWorkspaceMention,
    workspaceMention,
    workspaceMentionResults,
    workspaceMentionSelection,
  ]);
  const insertSuggestion = (suggestion: string) => {
    applyProgrammatic(suggestion);
    requestAnimationFrame(() => {
      inputRef.current?.focus();
    });
  };

  const insertCommand = (command: string) => {
    applyProgrammatic(command);
    setCommandFocusSignal((signal) => signal + 1);
  };

  // Bare `/model` is the daemon's own "open the picker" command — the resulting overlay comes
  // back on `Snapshot.overlay` and OverlayHost renders it, so nothing has to be replicated here.
  const openModelPicker = () => {
    if (!send({ kind: "prompt", text: "/model" })) {
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  };

  const openModePicker = () => {
    if (!send({ kind: "prompt", text: "/mode" })) {
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  };

  useAppShortcut("composer.effort", () => setEffortVisible(true));
  useAppShortcut("composer.temper", openModePicker);
  useAppShortcut("composer.model", openModelPicker);
  const openModelPickerRef = useRef(openModelPicker);
  useEffect(() => {
    openModelPickerRef.current = openModelPicker;
  });
  // A pane header's model chip asks for this session's picker.
  useEffect(
    () => onModelPickerRequest((id) => {
      if (id === sessionId) openModelPickerRef.current();
    }),
    [sessionId],
  );
  useAppShortcut("composer.plan", () => insertCommand("/plan "));

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
    reviewComments.length === 0 &&
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

  const commit = (value: string, invert = false) => {
    const trimmed = value.trim();
    if (!trimmed && reviewComments.length === 0 && visualAnnotations.length === 0) return;
    // An attachment still in flight (or failed) must never be silently dropped from the send —
    // block it with a toast instead so the user notices and can wait/remove/retry.
    if (attachments.some((a) => a.state === "uploading")) {
      toast.show("still uploading — wait a moment", { tone: "warn" });
      return;
    }
    if (attachments.some((a) => a.state === "error" || (a.state === "done" && !a.path))) {
      toast.show("an attachment failed — remove it or retry", { tone: "danger" });
      return;
    }
    // Only fully-uploaded attachments actually ride the daemon's next prompt (remote.rs
    // `RemoteInput::Attach`) — a failed upload never reached the session's pending queue, so it
    // must not be claimed on the sent bubble either.
    const sent: SentAttachment[] = attachments
      .filter((a) => a.state === "done")
      .map((a) => ({ id: a.id, name: a.name, image: a.image, uri: a.uri, path: a.path }));
    const prompt = formatVisualAnnotationsPrompt(
      formatReviewCommentsPrompt(trimmed, reviewComments),
      visualAnnotations,
    );
    const mode: FollowUpMode = busy ? effectiveMode(followUp, invert) : "steer";
    if (!onSend(prompt, sent, mode)) return;
    setLastPrompt(prompt);
    if (trimmed) sentRef.current = [...sentRef.current, trimmed].slice(-RECALL_SENT_MAX);
    editsRef.current = EMPTY_EDITS;
    recallRef.current = { state: RECALL_IDLE, shown: null };
    // Whatever suggestion is live right now belongs to the turn that's ending, not the one this
    // send just started — mask it so it can't flash back in once the draft clears below.
    if (suggestedPrompt) setSuppressedSuggestion(suggestedPrompt);
    haptics.sendPrompt();
    setText("");
    void clearDraft(sessionId);
    void clearDraftAttachments(sessionId);
    setAttachments([]);
    clearReviewComments(sessionId);
    clearVisualAnnotations(sessionId);
    if (Platform.OS === "web") setHeight(MIN_HEIGHT);
  };
  // `commit` closes over `onSend` (and whatever connection state it reads), so it's a new
  // function every render. The web keydown listener below is only bound once per session —
  // route it through this ref so Enter always calls the CURRENT commit/onSend, not the one
  // captured when the listener was attached (otherwise a mount-while-disconnected session
  // would queue Enter-sent messages offline forever, even after the WS reconnects).
  const commitRef = useRef(commit);
  const setTextRef = useRef(setText);
  const focusEndRef = useRef(focusEnd);
  useEffect(() => {
    commitRef.current = commit;
    setTextRef.current = setText;
    focusEndRef.current = focusEnd;
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
      if (!uploaded?.path) throw new Error("Upload completed without an attachment path");
      setAttachments((prev) =>
        prev.map((a) => (a.id === id ? { ...a, state: "done", path: uploaded.path } : a)),
      );
    } catch {
      setAttachments((prev) => prev.map((a) => (a.id === id ? { ...a, state: "error" } : a)));
    }
  };

  const runUploadRef = useRef(runUpload);
  useEffect(() => {
    runUploadRef.current = runUpload;
  });

  // Web/desktop drop target: files dragged over the composer upload like a picked file. Counting
  // enter/leave keeps the overlay steady while the drag crosses the composer's child elements.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = wrapRef.current as unknown as HTMLElement | null;
    if (!node) return;
    let depth = 0;
    const carriesFiles = (e: DragEvent) => e.dataTransfer?.types?.includes("Files") ?? false;
    const onEnter = (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      e.preventDefault();
      depth += 1;
      setDragging(true);
    };
    const onOver = (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    const onLeave = (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) setDragging(false);
    };
    const onDrop = (e: DragEvent) => {
      if (!carriesFiles(e)) return;
      e.preventDefault();
      depth = 0;
      setDragging(false);
      for (const file of filesFromDragEvent(e)) {
        void runUploadRef.current(pickedFromDroppedFile(file, URL.createObjectURL(file)), file);
      }
    };
    node.addEventListener("dragenter", onEnter);
    node.addEventListener("dragover", onOver);
    node.addEventListener("dragleave", onLeave);
    node.addEventListener("drop", onDrop);
    return () => {
      node.removeEventListener("dragenter", onEnter);
      node.removeEventListener("dragover", onOver);
      node.removeEventListener("dragleave", onLeave);
      node.removeEventListener("drop", onDrop);
    };
  }, []);

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
    applyProgrammatic(current.trim().length > 0 ? `${current} ${trimmed}` : trimmed);
  };

  // Web: Enter=send / Shift+Enter=newline + paste-image. Bound directly to the underlying DOM
  // node (RN-Web's TextInput ref IS that node) since RN's onKeyPress doesn't expose `shiftKey`.
  // `recording` is a required dep, not an oversight: below, `recording ? <VoiceRecordingPill/>
  // : <View>…<TextInput ref={inputRef}/>…</View>` unmounts the TextInput while recording and
  // remounts a BRAND NEW `<textarea>` node when it ends. Without `recording` here, the effect
  // never re-runs on that transition, so `node` stays pinned to the old, now-detached element —
  // the live textarea has no listeners at all. Keying on `recording` (not e.g. re-binding every
  // render) means it only re-runs on that mount/unmount edge, not on every keystroke.
  useEffect(() => {
    if (Platform.OS !== "web") return;
    const node = inputRef.current as unknown as HTMLTextAreaElement | null;
    if (!node) return;

    const onKeyDown = (e: KeyboardEvent) => {
      // An IME candidate-confirming Enter (CJK input) must only confirm the candidate, not send.
      if (e.isComposing) return;
      const mentionResults = workspaceMentionResultsRef.current;
      if (workspaceMentionRef.current && mentionResults.length > 0) {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const delta = e.key === "ArrowDown" ? 1 : -1;
          setWorkspaceMentionSelection((current) => {
            const next = (current + delta + mentionResults.length) % mentionResults.length;
            workspaceMentionSelectionRef.current = next;
            return next;
          });
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          const selected =
            mentionResults[workspaceMentionSelectionRef.current] ?? mentionResults[0];
          if (selected) insertWorkspaceMentionRef.current(selected.path);
          return;
        }
      }
      // Enter (no Shift) sends; ⌘/Ctrl+Enter sends too, even with Shift held, and mid-turn does
      // the opposite of the "Enter sends as" setting (steer <-> queue).
      if (e.key === "Enter" && (!e.shiftKey || e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        commitRef.current(textRef.current, e.metaKey || e.ctrlKey);
        return;
      }
      const plain = !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey;
      if (plain && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        const caret = node.selectionStart ?? 0;
        const step = recallStep(
          e.key === "ArrowUp" ? "up" : "down",
          recallEntriesRef.current,
          recallRef.current.state,
          textRef.current,
          caret,
          caret !== (node.selectionEnd ?? caret),
        );
        if (step.handled) {
          e.preventDefault();
          recallRef.current = { state: step.state, shown: step.text };
          textRef.current = step.text;
          setTextRef.current(step.text);
          focusEndRef.current(step.text);
        }
        return;
      }
      const mod = e.metaKey || e.ctrlKey;
      if (mod && !e.altKey && e.key.toLowerCase() === "z") {
        const redo = e.shiftKey;
        const result = (redo ? redoEdit : undoEdit)(editsRef.current, textRef.current);
        if (result) {
          e.preventDefault();
          editsRef.current = result.stack;
          textRef.current = result.text;
          setTextRef.current(result.text);
          focusEndRef.current(result.text);
        }
      } else if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "y") {
        const result = redoEdit(editsRef.current, textRef.current);
        if (result) {
          e.preventDefault();
          editsRef.current = result.stack;
          textRef.current = result.text;
          setTextRef.current(result.text);
          focusEndRef.current(result.text);
        }
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
  }, [sessionId, recording]);

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
      applyProgrammatic(suggestion);
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
  }, [showGhost, activeSuggestion, recording, applyProgrammatic]);

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

  const sendMode: FollowUpMode = busy ? effectiveMode(followUp, false) : "steer";
  const otherMode: FollowUpMode = sendMode === "steer" ? "queue" : "steer";
  const modKey = isMacOS ? "⌘" : "Ctrl";
  const previewUriFor = (a: { image: boolean; path?: string }) =>
    a.image && a.path && baseUrl
      ? `${baseUrl}/api/upload?session=${encodeURIComponent(sessionId)}&path=${encodeURIComponent(a.path)}`
      : undefined;
  const sideQuestion = /^\/(btw|side)(\s|$)/i.test(text);

  return (
    <View
      ref={wrapRef}
      style={[
        styles.wrap,
        {
          // Composer floats over the screen's own bg1 (Hearth: no full-bleed panel) — the
          // reply row below carries its own bg2/border pill per the prototype. `Screen`
          // omits "bottom" from its safe-area edges for this route (see index.tsx), so this
          // wrap still owns the home-indicator inset padding even though it's transparent.
          paddingBottom: space.space8 + insets.bottom,
        },
      ]}
    >
      {!recording && visualAnnotations.length > 0 ? (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.commandScroll}
          contentContainerStyle={styles.chipsRow}
          keyboardShouldPersistTaps="handled"
          accessibilityLabel="Pending browser preview annotations"
        >
          {visualAnnotations.map((annotation) => (
            <Chip
              key={annotation.id}
              icon={<MousePointer2 size={14} strokeWidth={1.75} color={tokens.accent} />}
              label={visualAnnotationLabel(annotation)}
              selected
              onPress={() => removeVisualAnnotation(sessionId, annotation.id)}
              testID={`visual-annotation-${annotation.id}`}
            />
          ))}
        </ScrollView>
      ) : null}

      {!recording && reviewComments.length > 0 ? (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={styles.commandScroll}
          contentContainerStyle={styles.chipsRow}
          keyboardShouldPersistTaps="handled"
          accessibilityLabel="Pending review annotations"
        >
          {reviewComments.map((comment) => (
            <Chip
              key={comment.id}
              icon={<MessageSquare size={14} strokeWidth={1.75} color={tokens.accent} />}
              label={`${comment.path} ${reviewRangeLabel(comment.side, comment.startLine, comment.endLine)}`}
              selected
              onPress={() => removeReviewComment(sessionId, comment.id)}
              testID={`review-comment-${comment.id}`}
            />
          ))}
        </ScrollView>
      ) : null}

      {!recording ? (
        <MentionChips text={text} onRemove={removeMentionChip} />
      ) : null}

      {!recording && attachments.length > 0 ? (
        <AttachmentTray attachments={attachments} onRemove={removeAttachment} previewUriFor={previewUriFor} />
      ) : null}

      {!recording && workspaceMention && (workspaceMentionQuery.length > 0 || contextMentions.length > 0) ? (
        <View
          style={[
            styles.mentionMenu,
            { backgroundColor: tokens.bg2, borderColor: tokens.border },
          ]}
          accessibilityRole="menu"
          accessibilityLabel="Workspace file suggestions"
        >
          {workspaceSearch.isLoading && workspaceMentionResults.length === 0 ? (
            <Text style={[type.meta, styles.mentionNotice, { color: tokens.ink3 }]}>
              searching workspace…
            </Text>
          ) : workspaceMentionResults.length > 0 ? (
            workspaceMentionResults.map((result, index) => (
              <MentionMenuRow
                key={result.path}
                result={result}
                selected={index === workspaceMentionSelection}
                onSelect={insertWorkspaceMention}
              />
            ))
          ) : workspaceSearch.isFetched ? (
            <Text style={[type.meta, styles.mentionNotice, { color: tokens.ink3 }]}>
              no matching workspace files
            </Text>
          ) : null}
        </View>
      ) : null}

      {!recording && (chipsOpen || text.startsWith("/") || (showSuggestionChip && activeSuggestion)) ? (
        <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.commandScroll} contentContainerStyle={styles.chipsRow} keyboardShouldPersistTaps="handled">
          {text.length === 0 ? <Chip size="compact" label="goal" icon={<Sparkles size={13} strokeWidth={1.75} color={tokens.ink3} />} onPress={() => setGoalVisible(true)} testID="chip-goal" /> : null}
          {text.length === 0 || text.startsWith("/") ? commandHints.map((cmd) => (
            <Chip size="compact" key={cmd} label={cmd} onPress={() => insertCommand(cmd)} testID={`chip-${cmd}`} />
          )) : null}
          {lastPrompt ? (
            <Chip
              size="compact"
              label="resend last"
              icon={<RotateCcw size={14} strokeWidth={1.75} color={tokens.ink3} />}
              onPress={() => commit(lastPrompt)}
              testID="resend-last-prompt"
            />
          ) : null}
          {showSuggestionChip && activeSuggestion ? (
            <Animated.View style={suggestionChipStyle}>
              <Chip
                size="compact"
                label={activeSuggestion}
                icon={<Sparkles size={14} strokeWidth={1.75} color={tokens.accent} />}
                onPress={() => insertSuggestion(activeSuggestion)}
                testID="suggested-prompt-chip"
              />
            </Animated.View>
          ) : null}
        </ScrollView>
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
        <View
          style={[
            styles.composerCard,
            {
              backgroundColor: tokens.bg2,
              borderColor: focused ? tokens.accent : tokens.borderStrong,
            },
            shadowStyle(depth.sheet),
          ]}
        >
          <View style={styles.metaRow}>
              {model ? (
                <MetaChip
                  // The daemon sends the literal sentinel "—" for an Automatic session (no
                  // model pinned yet) — same value StatusStrip already special-cases. A raw
                  // dash on the chip reads as broken/missing data, not as "automatic".
                  label={modelChipLabel(model, modelPinned, isCompact)}
                  icon={<ProviderGlyph provider={model} size={12} color={tokens.ink3} accessibilityLabel={null} />}
                  color={tokens.ink2}
                  accessibilityLabel={`model: ${modelChipLabel(model, modelPinned)} — change model`}
                  testID="composer-model-chip"
                  onPress={openModelPicker}
                />
              ) : null}
              {/* Absent `effort` is not missing data — it IS the session default ("auto", what
                  EffortPicker calls the default detent), so the chip stays reachable either
                  way rather than hiding the only way to open the picker. */}
              <MetaChip
                // A phone needs the width for the model's name; the gauge glyph says "effort".
                label={isCompact ? effort ?? "auto" : `effort · ${effort ?? "auto"}`}
                icon={isCompact ? <Gauge size={11} strokeWidth={1.75} color={tokens.ink4} /> : undefined}
                color={tokens.ink3}
                accessibilityLabel={`reasoning effort: ${effort ?? "auto"} — change effort`}
                testID="composer-effort-chip"
                // Short and informative ("effort · high"): a long model id used to squeeze it to
                // "effort …", hiding the one value the chip exists to show.
                fixed
                onPress={() => setEffortVisible(true)}
              />
              {/* Attach sits in this row's free space: a row of its own made the composer a
                  quarter of a phone screen, and squeezing it beside the input made short
                  replies wrap. */}
              {temper ? (
                <MetaChip
                  label={iconOnlyChips ? "" : temper.toLowerCase()}
                  icon={<ShieldHalf size={11} strokeWidth={1.75} color={tokens.ink4} />}
                  color={tokens.ink3}
                  accessibilityLabel={`mode: ${temper} — change mode`}
                  testID="composer-temper-chip"
                  fixed
                  bare={iconOnlyChips}
                  onPress={openModePicker}
                />
              ) : null}
              <View style={styles.metaSpacer} />
              <IconButton
                icon={<ImageIcon size={18} strokeWidth={1.75} color={tokens.ink3} />}
                onPress={onAttachImage}
                accessibilityLabel="attach photo"
                style={styles.metaIcon}
              />
              <IconButton
                icon={<FileText size={18} strokeWidth={1.75} color={tokens.ink3} />}
                onPress={onAttachDocument}
                accessibilityLabel="attach file"
                style={styles.metaIcon}
              />
          </View>
          <View
            style={[
              styles.row,
              {
                // Preserve the web rest-state centering, then pin controls beside the newest
                // line once it grows. Native height belongs to the mirror, so controls stay
                // bottom-aligned without feeding a measured height back through JavaScript.
                alignItems:
                  Platform.OS === "web"
                    ? height > MIN_HEIGHT
                      ? "flex-end"
                      : "center"
                    : "flex-end",
              },
            ]}
          >
          <View
            style={[
              styles.inputWrap,
              usesNativeMirror ? styles.nativeInputWrap : null,
            ]}
          >
            {usesNativeMirror ? (
              <NativeComposerMirror text={nativeText} color={tokens.ink} />
            ) : null}
            <TextInput
              ref={inputRef}
              {...(Platform.OS === "web" ? ({ dataSet: { focusZone: "composer" } } as object) : null)}
              value={Platform.OS === "web" ? text : nativeText}
              onChangeText={(next) => {
                recordComposerInput();
                if (recallRef.current.shown !== next) recallRef.current = { state: RECALL_IDLE, shown: null };
                if (Platform.OS !== "web") setNativeText(next);
                setText(next);
              }}
              {...imeProps}
              onFocus={onInputFocus}
              onBlur={onInputBlur}
              returnKeyType="default"
              autoCapitalize={text.startsWith("/") ? "none" : "sentences"}
              autoCorrect={!text.startsWith("/")}
              spellCheck={!text.startsWith("/")}
              multiline
              // Keep the browser textarea at one row initially; native must not receive this
              // constraint or Android treats the multiline composer as a single-line input.
              numberOfLines={Platform.OS === "web" ? 1 : undefined}
              scrollEnabled={composerScrollEnabled(Platform.OS, height)}
              textAlignVertical="top"
              // Hidden while the ghost is showing — it renders the same "empty state" copy
              // itself, and both at once would double up.
              placeholder={showGhost ? undefined : composerPlaceholder(model, isCompact)}
              placeholderTextColor={tokens.ink3}
              style={[
                type.body,
                styles.input,
                styles.inputTextMetrics,
                webInputTextStyle,
                { color: tokens.ink },
                Platform.OS === "web" ? [styles.webInputSize, { height }] : StyleSheet.absoluteFill,
              ]}
              accessibilityLabel="message"
              testID="composer-input"
            />
            {recognizedCommand ? <View pointerEvents="none" style={styles.commandRecognition}><Text style={[type.meta, { color: tokens.accent }]}>{leadingCommand} command</Text></View> : null}
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
          {action === "steer" ? (
            <IconButton
              icon={<Square size={14} strokeWidth={1.75} color={tokens.danger} fill={tokens.danger} />}
              onPress={onInterrupt}
              accessibilityLabel="stop"
              testID="composer-stop"
            />
          ) : null}
          <IconButton
            icon={<View style={styles.actionIcon}><Animated.View style={sendIconStyle}>{action === "steer" ? (sendMode === "queue" ? <ListEnd size={18} strokeWidth={2} color={tokens.onAccent} /> : <Zap size={18} strokeWidth={2} color={tokens.onAccent} fill={tokens.onAccent} />) : online ? <ArrowUp size={20} strokeWidth={2} color={canSend ? tokens.onAccent : tokens.ink4} /> : <Clock size={18} strokeWidth={1.75} color={canSend ? tokens.onAccent : tokens.ink4} />}</Animated.View><Animated.View style={[styles.actionLayer, stopIconStyle]}><Square size={16} strokeWidth={1.75} color={tokens.onAccent} fill={tokens.onAccent} /></Animated.View></View>}
            onPress={action === "stop" ? onInterrupt : () => commit(text)}
            disabled={action !== "stop" && !canSend}
            accessibilityLabel={
              action === "stop"
                ? "stop"
                : action === "steer"
                  ? sendMode === "queue"
                    ? "queue — runs after this turn"
                    : "steer — sent to the model at this turn's next step"
                  : online
                    ? "send"
                    : "queue — will send on reconnect"
            }
            style={[styles.sendCircle, { backgroundColor: action === "stop" ? tokens.danger : canSend ? tokens.accent : tokens.bg3 }]}
          />
          </View>
        </View>
      )}
      {!online && !recording ? (
        <Text style={[type.meta, styles.offlineHint, { color: tokens.ink3 }]}>will send on reconnect</Text>
      ) : action === "steer" && !recording ? (
        <View style={styles.hintRow}>
          <Text style={[type.meta, styles.hintText, { color: tokens.ink3 }]} numberOfLines={2}>
            {sendMode === "queue" ? "⏳ queue — runs after this turn" : "⚡ steer — the model gets this at its next step"}
            {Platform.OS === "web" && followUp.supported ? ` · ${modKey}↵ ${otherMode}s instead` : ""}
          </Text>
          {followUp.supported ? (
            <MiniPressable
              onPress={() => updateConfig.mutate({ key: FOLLOW_UP_KEY, value: otherMode, scope: "user" })}
              accessibilityRole="button"
              accessibilityLabel={`Enter sends as ${followUp.mode}. Switch to ${invertLabel(followUp.mode)}`}
              idleBorderColor={tokens.border}
              style={styles.hintToggle}
              testID="composer-followup-toggle"
            >
              <Text style={[type.monoMeta, { color: tokens.ink2 }]}>Enter: {followUp.mode}</Text>
            </MiniPressable>
          ) : null}
        </View>
      ) : sideQuestion && !recording ? (
        <Text style={[type.meta, styles.offlineHint, { color: tokens.ink3 }]}>side question — the answer is shown here and never joins the transcript</Text>
      ) : null}
      {dragging ? (
        <View
          pointerEvents="none"
          style={[styles.dropZone, { backgroundColor: tokens.selection, borderColor: tokens.accent }]}
          testID="composer-drop-zone"
        >
          <Upload size={18} strokeWidth={1.75} color={tokens.accent} />
          <Text style={[type.sub, { color: tokens.accent }]}>Drop to attach</Text>
        </View>
      ) : null}
      <GoalSheet visible={goalVisible} onClose={() => setGoalVisible(false)} onSubmit={commit} />
      {/* Headless: the composer's own chip is the trigger, so the picker contributes only its
          sheet/popover. Commits `/effort <level>` over the same prompt path. */}
      <EffortPicker
        effort={effort}
        send={send}
        visible={effortVisible}
        onClose={() => setEffortVisible(false)}
        showTrigger={false}
      />
    </View>
  );
}

/** Every prop the caller passes is either a primitive extracted from `snapshot`
 * (`busy`/`online`/`suggestedPrompt`/`model`/`effort`) or a `useCallback`-stabilized handler,
 * so this memo actually skips re-renders across the ~30ms WS snapshot frames that don't
 * change any of them (see `mobile/src/app/session/[id]/index.tsx`'s `handleSend`). */
export const Composer = React.memo(ComposerImpl);

function invertLabel(mode: FollowUpMode): FollowUpMode {
  return mode === "steer" ? "queue" : "steer";
}

/** One control-row chip: mono value + a chevron marking it as a picker, not a readout. */
function MetaChip({
  label,
  color,
  accessibilityLabel,
  testID,
  onPress,
  fixed = false,
  bare = false,
  icon,
}: {
  label: string;
  color: string;
  accessibilityLabel: string;
  testID: string;
  onPress: () => void;
  icon?: React.ReactNode;
  /** Keep the full label; the other chip in the row gives up the width instead. */
  fixed?: boolean;
  /** Icon only, without the picker chevron, for a phone-width row. */
  bare?: boolean;
}) {
  const { tokens } = useTheme();
  return (
    <Pressable
      onPress={onPress}
      hitSlop={8}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      testID={testID}
      style={[styles.metaChip, fixed && styles.metaChipFixed, { borderColor: tokens.border }]}
    >
      {icon}
      {label ? <Text style={[type.monoMeta, styles.metaChipLabel, { color }]} numberOfLines={1}>{label}</Text> : null}
      {bare ? null : <ChevronDown size={11} strokeWidth={1.75} color={tokens.ink4} />}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrap: {
    position: "relative",
    paddingHorizontal: space.space12,
    paddingTop: space.space8,
    paddingBottom: space.space8,
    gap: space.space8,
  },
  chipsRow: { flexDirection: "row", gap: space.space8, paddingRight: space.space12 },
  commandScroll: { flexGrow: 0, flexShrink: 0 },
  mentionMenu: {
    maxHeight: 260,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  mentionNotice: { paddingHorizontal: space.space12, paddingVertical: space.space8 },
  // D Main / M Session Chat composer: one bordered card (radius4, hairline) wrapping the
  // optional model/effort meta row and the icon+input control row beneath it.
  composerCard: {
    borderWidth: 1,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  metaRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingLeft: space.space8,
    paddingRight: space.space4,
    paddingTop: space.space4,
  },
  metaChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    minWidth: 0,
    flexShrink: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    paddingHorizontal: space.space8,
    paddingVertical: 3,
  },
  metaChipFixed: { flexShrink: 0 },
  metaChipLabel: { flexShrink: 1, minWidth: 0 },
  metaSpacer: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  metaIcon: { width: 32, height: 32 },
  row: {
    position: "relative",
    flexDirection: "row",
    gap: space.space4,
    paddingLeft: space.space4,
    paddingRight: space.space4,
    paddingVertical: space.space4,
  },
  input: {
    flex: 1,
    minWidth: 0,
    textAlignVertical: "top",
  },
  inputTextMetrics: {
    paddingHorizontal: space.space8,
    paddingVertical: composerInputVerticalPadding(Platform.OS),
  },
  // `input`'s flex: 1 has a 0% basis, and inside the column wrapper that basis overrides the
  // measured height, which pinned the web composer to one row however much text wrapped.
  webInputSize: { flexGrow: 0, flexShrink: 0, flexBasis: "auto" },
  nativeMirror: {
    width: "100%",
    opacity: 0,
  },
  nativeInputWrap: {
    minHeight: MIN_HEIGHT,
    maxHeight: MAX_HEIGHT,
    overflow: "hidden",
  },
  inputWrap: { flex: 1, minWidth: 0, position: "relative", overflow: "visible" },
  commandRecognition: { position: "absolute", right: space.space8, top: space.space8, backgroundColor: "transparent" },
  // Mirrors `input`'s own padding exactly so the ghost text lines up with where a typed
  // caret would sit — only ever shown while the TextInput is empty (see `suggestionActive`).
  ghostOverlay: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    paddingHorizontal: space.space8,
    paddingVertical: (MIN_HEIGHT - LINE_HEIGHT) / 2,
  },
  // Machined: square-ish accent send button, not a full circle — radius4 is the compressed
  // 3px step (see tokens.ts comment on the radii scale).
  sendCircle: { borderRadius: radii.radius4, width: tapTarget, height: tapTarget },
  actionIcon: { width: 20, height: 20, alignItems: "center", justifyContent: "center" },
  actionLayer: { position: "absolute" },
  offlineHint: { paddingLeft: space.space4 },
  hintRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingLeft: space.space4 },
  hintText: { flex: 1, minWidth: 0 },
  hintToggle: {
    flexShrink: 0,
    borderRadius: radii.radius4,
    paddingHorizontal: space.space8,
    paddingVertical: 2,
  },
  dropZone: {
    position: "absolute",
    top: space.space4,
    left: space.space12,
    right: space.space12,
    bottom: space.space4,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    borderWidth: 1,
    borderStyle: "dashed",
    borderRadius: radii.radius8,
    opacity: 0.94,
  },
});
