// Machined desktop shell — the bottom terminal dock (D Rail + Terminal, docs/design/machined
// INVENTORY.md L133-186). A real PTY on the daemon over `openTerminalSocket()`; the view is a
// bounded ANSI scrollback (see ansi.ts for why this is a hand-rolled log view and not xterm.js)
// rendered with react-native primitives, so it works identically in the Tauri webview and the
// browser build with no new dependency.
//
// Deviation from the frame, deliberate: the design draws the terminal's controls into its dock
// header, but DockHost's header is generic across every dock (usage/git/terminal) and has no
// route to this component's socket. The interrupt/clear affordances therefore live in a dense
// mono status strip at the foot of the dock instead.
import { Plus, RefreshCw, RotateCcw, Square, Terminal } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  NativeSyntheticEvent,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type LayoutChangeEvent,
  type NativeScrollEvent,
} from "react-native";

import {
  listTerminalSessions,
  openTerminalSocket,
  type TerminalSessionSummary,
  type TerminalSocket,
  type TerminalStatus,
} from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { HOST_CAP_CLOSE_CODE, reportHostCap } from "../../lib/hostStatus";
import { useTerminalPreferences, terminalLineHeight } from "../../lib/terminalPreferences";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { useOptionalWorkbench } from "../workbench/WorkbenchProvider";
import { AnsiScrollback, type AnsiLine } from "./ansi";
import { ESCAPE_SKIP_PROPS } from "../../lib/escapeSkip";
import { findFocusProps } from "../../lib/find/findController";
import { FindBar } from "../find/FindBar";
import { ansiPalette, TerminalLine } from "./TerminalLine";
import { clearSelection, copyText, hasAnySelection, hasDom, selectedTextWithin } from "./terminalClipboard";
import {
  closesItselfOnExit,
  compareTerminalIds,
  copyOrInterrupt,
  exitLabel,
  measuredCharWidth,
  neighbourTerminal,
  nextTerminalId,
  resizeDelay,
  terminalGrid,
  terminalInputDelta,
  terminalTitle,
} from "./terminalModel";
import { terminalsToAdopt } from "./terminalTabs";
import { useTerminalActivity } from "./useTerminalActivity";
import { useTerminalFind } from "./useTerminalFind";
import { useTerminalLinkOpener } from "./useTerminalLinkOpener";

const MAX_SCROLLBACK_LINES = 2000;
/** Output is coalesced into at most one re-render per frame-ish interval. */
const FLUSH_MS = 33;
/** Geist Mono advance width is ≈ 0.6em; used until a hidden probe has measured the real glyph. */
const ESTIMATED_CHAR_RATIO = 0.6;
const PROBE_TEXT = "M".repeat(40);
const PROBE_FALLBACK_MS = 400;
const PADDING_H = 14;
/** A reattached full-screen program repaints on a size change, not on a repeated size. */
const WIGGLE_DELAY_MS = 250;
const WIGGLE_RESTORE_MS = 80;
const COPIED_FLASH_MS = 1_200;
const WHEEL_LINE_PX = 16;

const CONTROL_KEYS: Record<string, string> = {
  Enter: "\r",
  Backspace: "\x7f",
  Tab: "\t",
  Escape: "\x1b",
  ArrowUp: "\x1b[A",
  ArrowDown: "\x1b[B",
  ArrowRight: "\x1b[C",
  ArrowLeft: "\x1b[D",
  Delete: "\x1b[3~",
  Home: "\x1b[H",
  End: "\x1b[F",
  PageUp: "\x1b[5~",
  PageDown: "\x1b[6~",
};

const COMPACT_KEYS = [
  { label: "esc", value: "\x1b", accessibilityLabel: "Escape" },
  { label: "tab", value: "\t", accessibilityLabel: "Tab" },
  { label: "←", value: "\x1b[D", accessibilityLabel: "Left arrow" },
  { label: "↑", value: "\x1b[A", accessibilityLabel: "Up arrow" },
  { label: "↓", value: "\x1b[B", accessibilityLabel: "Down arrow" },
  { label: "→", value: "\x1b[C", accessibilityLabel: "Right arrow" },
  { label: "^C", value: "\x03", accessibilityLabel: "Control C" },
  { label: "^L", value: "\x0c", accessibilityLabel: "Control L" },
] as const;

// ---------------------------------------------------------------------------
// Dock
// ---------------------------------------------------------------------------

export interface TerminalDockProps {
  sessionId: string | null;
  /** Stable daemon terminal identity. Desktop workbench tabs pin this explicitly. */
  terminalId?: string;
  /** Adds terminal tabs and touch-first modifier/navigation keys for compact layouts. */
  compact?: boolean;
  /** Desktop workbench callback used by the + control to open another resource tab. */
  onOpenTerminal?: (terminalId: string) => void;
  /** The workbench tab showing this terminal, kept selected while other terminals' tabs are restored. */
  surfaceId?: string | null;
  /** The shell ended on its own with code 0: the host closes the tab. */
  onShellExit?: () => void;
}

type ConnectionStatus = "idle" | "connecting" | "connected" | "disconnected";

const TERMINAL_OPEN_DEADLINE_MS = 15_000;

export function TerminalDock({
  sessionId,
  terminalId = "term-1",
  compact = false,
  onOpenTerminal,
  surfaceId = null,
  onShellExit,
}: TerminalDockProps) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const palette = useMemo(() => ansiPalette(tokens), [tokens]);
  const workbench = useOptionalWorkbench();
  const preferences = useTerminalPreferences();
  const fontSize = preferences.fontSize;
  const lineHeight = terminalLineHeight(fontSize);
  const openLink = useTerminalLinkOpener(sessionId);

  const bufferRef = useRef(new AnsiScrollback(MAX_SCROLLBACK_LINES));
  const socketRef = useRef<TerminalSocket | null>(null);
  const scrollRef = useRef<ScrollView | null>(null);
  const dockRef = useRef<View | null>(null);
  const lastCopyAtRef = useRef<number | null>(null);
  const attachedBeforeRef = useRef(new Set<string>());
  const stickyRef = useRef(true);
  const inputRef = useRef<TextInput | null>(null);
  // The hidden capture input's own last known text — see terminalInputDelta for why this is
  // tracked instead of assuming the native buffer actually clears back to "" between keystrokes.
  const lastNativeTextRef = useRef("");
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const connectedRef = useRef(false);
  const restartRef = useRef(false);
  const suppressReconnectRef = useRef(false);
  const terminalStatusRef = useRef<TerminalStatus>("running");
  const cleanExitRef = useRef<() => void>(() => undefined);

  const [lines, setLines] = useState<AnsiLine[]>([]);
  const { highlights, activeKey, activeLineRef } = useTerminalFind({
    buffer: bufferRef,
    scroll: scrollRef,
    input: inputRef,
    sticky: stickyRef,
    lineHeight,
  });
  const [focused, setFocused] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("idle");
  const [terminalStatus, setTerminalStatus] = useState<TerminalStatus>("running");
  const [exitCode, setExitCode] = useState<number | null>(null);
  const [copied, setCopied] = useState(false);
  const [box, setBox] = useState<{ width: number; height: number } | null>(null);
  const [charWidth, setCharWidth] = useState<number | null>(null);
  const [localTerminalId, setLocalTerminalId] = useState(terminalId);
  const [terminals, setTerminals] = useState<TerminalSessionSummary[]>([]);
  const [connectionGeneration, setConnectionGeneration] = useState(0);
  const effectiveTerminalId = compact ? localTerminalId : terminalId;
  const activityView = useTerminalActivity(sessionId, effectiveTerminalId, compact);
  const terminalsRef = useRef<TerminalSessionSummary[]>([]);
  useEffect(() => {
    terminalsRef.current = terminals;
  }, [terminals]);

  useEffect(() => {
    if (charWidth != null) return;
    const timer = setTimeout(() => setCharWidth(fontSize * ESTIMATED_CHAR_RATIO), PROBE_FALLBACK_MS);
    return () => clearTimeout(timer);
  }, [charWidth, fontSize]);

  const size = useMemo(
    () =>
      box && charWidth != null
        ? terminalGrid({ ...box, charWidth, lineHeight, paddingX: PADDING_H })
        : null,
    [box, charWidth, lineHeight],
  );
  const cols = size?.cols;
  const rows = size?.rows;

  useEffect(() => {
    if (!compact) setLocalTerminalId(terminalId);
  }, [compact, terminalId]);

  const refreshTerminals = useCallback(async () => {
    if (!baseUrl || !sessionId) {
      setTerminals([]);
      return;
    }
    try {
      setTerminals(await listTerminalSessions(baseUrl, sessionId));
    } catch {
      // The live terminal remains usable when metadata refresh fails.
    }
  }, [baseUrl, sessionId]);

  useEffect(() => {
    void refreshTerminals();
  }, [refreshTerminals]);

  const terminalIds = useMemo(
    () =>
      Array.from(new Set([...terminals.map((terminal) => terminal.terminal_id), effectiveTerminalId]))
        .sort(compareTerminalIds),
    [effectiveTerminalId, terminals],
  );

  // Mirrors `size` for the socket-open effect below, which must not re-run (and re-spawn the
  // pty) every time the pane is resized. Declared first so it is already up to date when that
  // effect runs in the same commit.
  const sizeRef = useRef<{ cols: number; rows: number } | null>(null);
  useEffect(() => {
    sizeRef.current = size;
  }, [size]);

  // Output arrives in bursts; one re-render per FLUSH_MS is enough and keeps a `cargo test`
  // firehose from re-rendering the list per frame.
  const flushTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scheduleFlush = useCallback(() => {
    if (flushTimer.current != null) return;
    flushTimer.current = setTimeout(() => {
      flushTimer.current = null;
      setLines(bufferRef.current.lines());
    }, FLUSH_MS);
  }, []);
  useEffect(
    () => () => {
      if (flushTimer.current != null) clearTimeout(flushTimer.current);
    },
    [],
  );

  const onLayout = useCallback((event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    setBox((previous) =>
      previous && previous.width === width && previous.height === height ? previous : { width, height },
    );
  }, []);

  const onProbeLayout = useCallback(
    (event: LayoutChangeEvent) => {
      setCharWidth(
        measuredCharWidth(event.nativeEvent.layout.width, PROBE_TEXT.length, fontSize * ESTIMATED_CHAR_RATIO),
      );
    },
    [fontSize],
  );

  // One attachment per terminal identity, opened once the pane has been measured so the PTY
  // starts at the right geometry. Closing this socket only detaches; the daemon owns the shell.
  const measured = cols != null;
  useEffect(() => {
    if (!baseUrl || !sessionId || !measured) return;
    suppressReconnectRef.current = false;
    const buffer = bufferRef.current;
    buffer.clear();
    setLines(buffer.lines());
    setConnectionStatus("connecting");
    connectedRef.current = false;
    let socket: TerminalSocket;
    let disposed = false;
    const restart = restartRef.current;
    restartRef.current = false;
    terminalStatusRef.current = "running";
    setTerminalStatus("running");
    setExitCode(null);
    const knownSequence =
      terminalsRef.current.find((terminal) => terminal.terminal_id === effectiveTerminalId)?.sequence ?? 0;
    const reattach = !restart && (attachedBeforeRef.current.has(effectiveTerminalId) || knownSequence > 0);
    attachedBeforeRef.current.add(effectiveTerminalId);
    let wiggleTimer: ReturnType<typeof setTimeout> | null = null;
    let wiggleRestore: ReturnType<typeof setTimeout> | null = null;
    try {
      socket = openTerminalSocket(
        baseUrl,
        sessionId,
        {
          onOutput: (chunk) => {
            if (disposed) return;
            buffer.write(chunk);
            scheduleFlush();
          },
          onStatus: (nextStatus, code) => {
            if (disposed) return;
            const wasRunning = terminalStatusRef.current === "running";
            terminalStatusRef.current = nextStatus;
            setTerminalStatus(nextStatus);
            setExitCode(nextStatus === "exited" ? code : null);
            void refreshTerminals();
            if (nextStatus === "exited" && wasRunning && closesItselfOnExit(code)) {
              cleanExitRef.current();
            }
          },
          onClear: () => {
            if (disposed) return;
            buffer.clear();
            setLines(buffer.lines());
          },
          onOpen: () => {
            if (disposed) return;
            connectedRef.current = true;
            setConnectionStatus("connected");
            void refreshTerminals();
            if (reattach) {
              // A full-screen agent redraws on a size change, not when the same size arrives
              // again, so nudge the width by one column and put it back.
              wiggleTimer = setTimeout(() => {
                const current = sizeRef.current;
                if (disposed || !current) return;
                socket.resize(current.cols + 1, current.rows);
                wiggleRestore = setTimeout(() => socket.resize(current.cols, current.rows), WIGGLE_RESTORE_MS);
              }, WIGGLE_DELAY_MS);
            }
          },
          onClose: (closed) => {
            // A socket this effect already replaced must not speak for the new one.
            if (disposed) return;
            if (closed.code === HOST_CAP_CLOSE_CODE) reportHostCap(closed.reason);
            connectedRef.current = false;
            setConnectionStatus("disconnected");
            void refreshTerminals();
            if (
              !disposed
              && !suppressReconnectRef.current
              && terminalStatusRef.current === "running"
            ) {
              reconnectTimerRef.current = setTimeout(
                () => setConnectionGeneration((generation) => generation + 1),
                1_500,
              );
            }
          },
          onError: () => {
            if (!disposed) setConnectionStatus("disconnected");
          },
        },
        {
          terminalId: effectiveTerminalId,
          size: sizeRef.current ?? undefined,
          restart,
        },
      );
    } catch (err) {
      // Transports reject an unroutable socket synchronously (an un-enrolled Anywhere host, a
      // malformed base URL). Uncaught, that unmounts the entire app via the root ErrorBoundary —
      // report it in the pane the user is already looking at instead.
      setConnectionStatus("disconnected");
      buffer.write(`\r\n[terminal unavailable: ${err instanceof Error ? err.message : String(err)}]\r\n`);
      scheduleFlush();
      return;
    }
    socketRef.current = socket;
    // An open request lost on the way — the relay dropped between the phone and the host — never
    // resolves: no open, no close, so the pane sat on "connecting" until the user found the
    // restart button. Give the attach a deadline and try again.
    const openDeadline = setTimeout(() => {
      if (disposed || connectedRef.current) return;
      // This timer owns the retry; keep the close handler from scheduling a second one.
      suppressReconnectRef.current = true;
      socket.close();
      setConnectionStatus("disconnected");
      reconnectTimerRef.current = setTimeout(
        () => setConnectionGeneration((generation) => generation + 1),
        1_500,
      );
    }, TERMINAL_OPEN_DEADLINE_MS);
    return () => {
      disposed = true;
      clearTimeout(openDeadline);
      if (wiggleTimer != null) clearTimeout(wiggleTimer);
      if (wiggleRestore != null) clearTimeout(wiggleRestore);
      if (reconnectTimerRef.current != null) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
      socketRef.current = null;
      socket.close();
    };
  }, [
    baseUrl,
    connectionGeneration,
    effectiveTerminalId,
    measured,
    refreshTerminals,
    scheduleFlush,
    sessionId,
  ]);

  // A large buffer makes every intermediate size of a drag expensive for the shell to repaint,
  // so a resize waits for the drag to settle.
  useEffect(() => {
    if (cols == null || rows == null) return;
    const timer = setTimeout(
      () => socketRef.current?.resize(cols, rows),
      resizeDelay(bufferRef.current.lines().length),
    );
    return () => clearTimeout(timer);
  }, [cols, rows]);

  useEffect(() => {
    if (compact || !workbench || !sessionId) return;
    const missing = terminalsToAdopt(workbench.state.bottom.tabs, sessionId, terminals);
    if (missing.length === 0) return;
    for (const id of missing) {
      workbench.openSurface({ kind: "terminal", sessionId, resourceId: id, title: terminalTitle(id) });
    }
    if (surfaceId) workbench.activateSurface("bottom", surfaceId);
  }, [compact, sessionId, surfaceId, terminals, workbench]);

  useEffect(() => {
    if (!hasDom || preferences.scrollSpeed === 1) return;
    const node = (scrollRef.current as unknown as { getScrollableNode?: () => HTMLElement } | null)?.getScrollableNode?.();
    if (!node) return;
    const speed = preferences.scrollSpeed;
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.deltaY === 0) return;
      event.preventDefault();
      const unit = event.deltaMode === 1 ? WHEEL_LINE_PX : 1;
      node.scrollTop += event.deltaY * unit * speed;
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => node.removeEventListener("wheel", onWheel);
  }, [preferences.scrollSpeed, sessionId]);

  const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    stickyRef.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - lineHeight;
  }, [lineHeight]);

  const onContentSizeChange = useCallback(() => {
    if (stickyRef.current) scrollRef.current?.scrollToEnd({ animated: false });
  }, []);

  const send = useCallback((data: string) => {
    socketRef.current?.send(data);
    stickyRef.current = true;
  }, []);

  const clear = useCallback(() => {
    if (!socketRef.current?.clear()) {
      bufferRef.current.clear();
      setLines(bufferRef.current.lines());
    }
  }, []);

  const reconnect = useCallback(() => {
    suppressReconnectRef.current = false;
    setConnectionGeneration((generation) => generation + 1);
  }, []);

  const restart = useCallback(() => {
    restartRef.current = true;
    suppressReconnectRef.current = false;
    terminalStatusRef.current = "running";
    setTerminalStatus("running");
    setConnectionGeneration((generation) => generation + 1);
  }, []);

  const stop = useCallback(() => {
    suppressReconnectRef.current = true;
    socketRef.current?.kill();
  }, []);

  // A shell that ended with code 0 is done: discard it and move on. Any other ending stays on
  // screen with its code and a Restart button.
  const handleCleanExit = useCallback(() => {
    if (compact) {
      const next = neighbourTerminal(terminalIds, effectiveTerminalId);
      if (!next) return;
      socketRef.current?.kill();
      setLocalTerminalId(next);
      return;
    }
    socketRef.current?.kill();
    onShellExit?.();
  }, [compact, effectiveTerminalId, onShellExit, terminalIds]);
  useEffect(() => {
    cleanExitRef.current = handleCleanExit;
  }, [handleCleanExit]);

  // Ctrl/Cmd+C copies a selection; pressing it again within half a second interrupts anyway.
  // Handled on the dock itself, in the capture phase, so it works whether the keyboard focus is
  // in the hidden input or on the selected text.
  useEffect(() => {
    if (!hasDom) return;
    const node = dockRef.current as unknown as HTMLElement | null;
    if (!node?.addEventListener) return;
    let flash: ReturnType<typeof setTimeout> | null = null;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== "c") return;
      const text = selectedTextWithin(node);
      if (event.shiftKey) {
        if (!text) return;
        event.preventDefault();
        event.stopPropagation();
        void copyText(text);
        return;
      }
      const action = copyOrInterrupt({
        meta: event.metaKey && !event.ctrlKey,
        hasSelection: text !== "",
        now: Date.now(),
        lastCopyAt: lastCopyAtRef.current,
      });
      if (action === "ignore") return;
      event.preventDefault();
      event.stopPropagation();
      if (action === "copy") {
        lastCopyAtRef.current = Date.now();
        void copyText(text).then((ok) => {
          if (!ok) return;
          setCopied(true);
          if (flash != null) clearTimeout(flash);
          flash = setTimeout(() => setCopied(false), COPIED_FLASH_MS);
        });
        return;
      }
      lastCopyAtRef.current = null;
      clearSelection();
      socketRef.current?.send("\x03");
      stickyRef.current = true;
      inputRef.current?.focus();
    };
    node.addEventListener("keydown", onKeyDown, true);
    return () => {
      node.removeEventListener("keydown", onKeyDown, true);
      if (flash != null) clearTimeout(flash);
    };
  }, []);

  const focusInput = useCallback(() => {
    // Focusing the input would drop a selection the person just made to copy it.
    if (hasAnySelection()) return;
    inputRef.current?.focus();
  }, []);

  const openNewTerminal = useCallback(() => {
    const next = nextTerminalId(terminalIds);
    if (!next) return;
    if (compact) setLocalTerminalId(next);
    else onOpenTerminal?.(next);
  }, [compact, onOpenTerminal, terminalIds]);

  const onKeyPress = useCallback(
    (event: NativeSyntheticEvent<{ key: string }>) => {
      // RN's typed payload is `{ key }`; the web runtime forwards the DOM modifiers on the
      // same object, which is the only route to ctrl-combos through a TextInput. Where they
      // are absent the combo simply does not fire — the ^C button below is the guaranteed path.
      const native = event.nativeEvent as { key: string; ctrlKey?: boolean; defaultPrevented?: boolean };
      const key = native.key;
      // An app shortcut (Mod+F opens Find) claimed this combo before the shell could see it.
      if (native.defaultPrevented) return;
      if (native.ctrlKey && key.length === 1) {
        // Ctrl+C is the dock-level handler's (copy vs interrupt); on web it never gets here.
        if (hasDom && key.toLowerCase() === "c") return;
        const code = key.toUpperCase().charCodeAt(0);
        if (code >= 64 && code <= 95) send(String.fromCharCode(code - 64));
        return;
      }
      // Printable characters arrive through onChangeText — handling them here too would
      // double-send every keystroke.
      if (key.length === 1) return;
      const mapped = CONTROL_KEYS[key];
      if (mapped) {
        send(mapped);
        // A control key handled here (notably Backspace on an IME that reports it this way
        // instead of shrinking onChangeText's text) never touched the tracked buffer above —
        // reset it too, so the next onChangeText diffs from the buffer's real state.
        lastNativeTextRef.current = "";
        inputRef.current?.clear();
      }
    },
    [send],
  );

  // Android delivers the soft keyboard's Enter on a single-line input as a submit, never as an
  // onKeyPress "Enter", so without this the shell never received a newline. iOS and web do send
  // the keypress, which already wrote "\r" above.
  const onSubmitEditing = useCallback(() => {
    if (Platform.OS !== "android") return;
    send(CONTROL_KEYS.Enter);
    lastNativeTextRef.current = "";
    inputRef.current?.clear();
  }, [send]);

  const onChangeText = useCallback(
    (text: string) => {
      // See terminalInputDelta: the native buffer is not guaranteed to have actually reset to ""
      // since the last keystroke, so `text` may be just the new character OR the whole
      // accumulated buffer — either way, diffing against what we last saw yields the right thing
      // to send exactly once.
      const delta = terminalInputDelta(lastNativeTextRef.current, text);
      lastNativeTextRef.current = text;
      if (delta) send(delta);
      // Best-effort: encourage the native buffer back to empty so it doesn't grow without bound
      // over a long typing session. Correctness does not depend on this actually taking effect
      // before the next keystroke — terminalInputDelta handles either outcome.
      inputRef.current?.clear();
      // A DOM input really is empty after clear(). Without this, the next keystroke of the same
      // character ("ll", "//") reads as unchanged text and is dropped.
      if (hasDom) lastNativeTextRef.current = "";
    },
    [send],
  );

  const caretColor =
    focused && connectionStatus === "connected" && terminalStatus === "running"
      ? tokens.accent
      : null;

  if (!sessionId) {
    return (
      <View style={styles.empty}>
        <EmptyState icon={Terminal} message="Open a session to run a terminal in its working directory." />
      </View>
    );
  }

  return (
    <View ref={dockRef} style={styles.dock} {...findFocusProps("terminal")}>
      <FindBar targetId="terminal" />
      {compact ? (
        <View style={[styles.tabs, { borderBottomColor: tokens.border }]}>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.tabsContent}
          >
            {terminalIds.map((id) => {
              const summary = terminals.find((terminal) => terminal.terminal_id === id);
              const active = id === effectiveTerminalId;
              const unseen = activityView.isUnseen(id);
              const streaming = activityView.isStreaming(id);
              return (
                <Pressable
                  key={id}
                  onPress={() => setLocalTerminalId(id)}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: active }}
                  style={[
                    styles.tab,
                    styles.tabRow,
                    {
                      borderBottomColor: active ? tokens.accent : "transparent",
                      backgroundColor: active ? hexToRgba(tokens.accent, 0.08) : "transparent",
                    },
                  ]}
                >
                  <Text style={[typeScale.monoMeta, { color: active ? tokens.ink : tokens.ink3 }]}>
                    {terminalTitle(id)}
                    {summary?.status === "exited" ? ` · ${exitLabel(summary.exit_code)}` : ""}
                  </Text>
                  {streaming ? (
                    <Text style={[typeScale.monoMeta, styles.liveBadge, { color: tokens.accent, backgroundColor: hexToRgba(tokens.accent, 0.12) }]}>
                      live
                    </Text>
                  ) : unseen ? (
                    <View
                      accessibilityLabel="New output"
                      style={[styles.unseenDot, { backgroundColor: tokens.accent }]}
                    />
                  ) : null}
                </Pressable>
              );
            })}
          </ScrollView>
          <Pressable
            onPress={openNewTerminal}
            disabled={nextTerminalId(terminalIds) == null}
            accessibilityRole="button"
            accessibilityLabel="New terminal"
            style={styles.tabAdd}
          >
            <Plus size={15} strokeWidth={1.75} color={tokens.ink3} />
          </Pressable>
        </View>
      ) : null}
      <Pressable style={styles.body} onPress={focusInput} accessibilityRole="none" {...ESCAPE_SKIP_PROPS}>
        <ScrollView
          ref={scrollRef}
          style={styles.scroll}
          contentContainerStyle={styles.scrollContent}
          onLayout={onLayout}
          onScroll={onScroll}
          onContentSizeChange={onContentSizeChange}
          scrollEventThrottle={64}
          showsVerticalScrollIndicator={false}
        >
          {lines.map((line, index) => (
            <TerminalLine
              key={line.key}
              line={line}
              palette={palette}
              caretColor={index === lines.length - 1 ? caretColor : null}
              fontSize={fontSize}
              lineHeight={lineHeight}
              onOpenLink={openLink}
              highlights={highlights.get(line.key)}
              activeRef={line.key === activeKey ? activeLineRef : undefined}
            />
          ))}
        </ScrollView>
        <Text
          pointerEvents="none"
          numberOfLines={1}
          onLayout={onProbeLayout}
          style={[styles.probe, { fontSize, lineHeight, fontFamily: monoFamily.regular }]}
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        >
          {PROBE_TEXT}
        </Text>
        {terminalStatus === "exited" ? (
          <View style={[styles.exitBanner, { borderTopColor: tokens.border, backgroundColor: tokens.bg2 }]}>
            <Text
              style={[typeScale.monoMeta, styles.exitText, { color: exitCode ? tokens.danger : tokens.ink3 }]}
              numberOfLines={2}
            >
              {exitCode == null ? "Process ended" : `Process exited with code ${exitCode}`}
            </Text>
            <Pressable
              onPress={restart}
              accessibilityRole="button"
              accessibilityLabel="Restart terminal"
              style={({ pressed }) => [
                styles.restartButton,
                { borderColor: tokens.accent, backgroundColor: hexToRgba(tokens.accent, pressed ? 0.22 : 0.1) },
              ]}
            >
              <RotateCcw size={12} strokeWidth={1.75} color={tokens.accent} />
              <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>Restart</Text>
            </Pressable>
          </View>
        ) : null}
        <TextInput
          ref={inputRef}
          // Deliberately uncontrolled (no `value` prop): a literal `value=""` fights Android's
          // native EditText for ownership of its own buffer on every re-render, which is what
          // let keystrokes accumulate into the native side instead of actually clearing —
          // onChangeText's delta computation (terminalInputDelta) is what keeps this correct now,
          // not a controlled value forcing a reset.
          onChangeText={onChangeText}
          onKeyPress={onKeyPress}
          onSubmitEditing={onSubmitEditing}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          style={styles.input}
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          submitBehavior="submit"
          accessibilityLabel="Terminal input"
          accessibilityHint="Types straight into the session's shell"
        />
      </Pressable>
      {compact ? (
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          style={[styles.compactControls, { borderTopColor: tokens.border }]}
          contentContainerStyle={styles.compactControlsContent}
          keyboardShouldPersistTaps="always"
        >
          {COMPACT_KEYS.map((key) => (
            <Pressable
              key={key.label}
              onPress={() => send(key.value)}
              accessibilityRole="button"
              accessibilityLabel={key.accessibilityLabel}
              style={({ pressed }) => [
                styles.compactKey,
                { borderColor: tokens.border },
                pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
              ]}
            >
              <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>{key.label}</Text>
            </Pressable>
          ))}
          <Pressable
            onPress={() => inputRef.current?.focus()}
            accessibilityRole="button"
            accessibilityLabel="Focus terminal keyboard"
            style={({ pressed }) => [
              styles.compactKey,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>keyboard</Text>
          </Pressable>
        </ScrollView>
      ) : null}
      <View style={[styles.status, { borderTopColor: tokens.border }]}>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>
          {connectionStatus}
          {terminalStatus === "exited" ? ` · ${exitLabel(exitCode)}` : ""}
          {size ? ` · ${size.cols}×${size.rows}` : ""}
          {copied ? " · copied" : ""}
        </Text>
        <View style={styles.statusActions}>
          {!compact && onOpenTerminal ? (
            <Pressable
              onPress={openNewTerminal}
              disabled={nextTerminalId(terminalIds) == null}
              accessibilityRole="button"
              accessibilityLabel="New terminal"
              style={({ pressed }) => [
                styles.statusButton,
                { borderColor: tokens.border },
                pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
              ]}
            >
              <Plus size={13} strokeWidth={1.75} color={tokens.ink3} />
            </Pressable>
          ) : null}
          {connectionStatus === "disconnected" ? (
            <Pressable
              onPress={reconnect}
              accessibilityRole="button"
              accessibilityLabel="Reconnect to terminal"
              style={({ pressed }) => [
                styles.statusButton,
                { borderColor: tokens.border },
                pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
              ]}
            >
              <RefreshCw size={13} strokeWidth={1.75} color={tokens.ink3} />
            </Pressable>
          ) : null}
          <Pressable
            onPress={() => send("\x03")}
            accessibilityRole="button"
            accessibilityLabel="Send interrupt"
            accessibilityHint="Control C"
            style={({ pressed }) => [
              styles.statusButton,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>^C</Text>
          </Pressable>
          <Pressable
            onPress={clear}
            accessibilityRole="button"
            accessibilityLabel="Clear terminal history"
            style={({ pressed }) => [
              styles.statusButton,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>clear</Text>
          </Pressable>
          <Pressable
            onPress={restart}
            accessibilityRole="button"
            accessibilityLabel="Restart terminal"
            style={({ pressed }) => [
              styles.statusButton,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <RotateCcw size={13} strokeWidth={1.75} color={tokens.ink3} />
          </Pressable>
          <Pressable
            onPress={stop}
            accessibilityRole="button"
            accessibilityLabel="Stop terminal process"
            style={({ pressed }) => [
              styles.statusButton,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.danger, 0.12) },
            ]}
          >
            <Square size={12} strokeWidth={1.75} color={tokens.danger} />
          </Pressable>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  dock: { flex: 1 },
  empty: { flex: 1, justifyContent: "center" },
  tabs: {
    height: 36,
    flexDirection: "row",
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  tabsContent: { alignItems: "stretch" },
  tab: {
    minWidth: 92,
    justifyContent: "center",
    paddingHorizontal: space.space12,
    borderBottomWidth: 2,
  },
  tabAdd: { width: 40, alignItems: "center", justifyContent: "center" },
  body: { flex: 1 },
  scroll: { flex: 1 },
  scrollContent: { paddingHorizontal: PADDING_H, paddingVertical: space.space8 },
  probe: { position: "absolute", left: 0, top: 0, opacity: 0 },
  exitBanner: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    paddingHorizontal: PADDING_H,
    paddingVertical: space.space8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  exitText: { flex: 1 },
  restartButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    height: 26,
    paddingHorizontal: space.space12,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 3,
  },
  tabRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  unseenDot: { width: 6, height: 6, borderRadius: 3 },
  liveBadge: { paddingHorizontal: 4, borderRadius: 2, overflow: "hidden" },
  // The input is a keystroke sink, never a visible field: the pty echoes what it accepts, so
  // rendering the draft locally would double every character.
  input: { position: "absolute", width: 1, height: 1, opacity: 0 },
  compactControls: {
    flexGrow: 0,
    flexShrink: 0,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  compactControlsContent: {
    minHeight: 38,
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space8,
  },
  compactKey: {
    minWidth: 34,
    height: 28,
    paddingHorizontal: space.space8,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 3,
  },
  status: {
    minHeight: 28,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: PADDING_H,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  statusActions: { flexDirection: "row", gap: space.space8 },
  statusButton: {
    minWidth: 26,
    height: 22,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 2,
    paddingHorizontal: 5,
  },
});
