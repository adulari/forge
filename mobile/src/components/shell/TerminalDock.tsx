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
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type TextStyle,
} from "react-native";

import {
  listTerminalSessions,
  openTerminalSocket,
  type TerminalSessionSummary,
  type TerminalSocket,
  type TerminalStatus,
} from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space, type ColorTokens } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { AnsiScrollback, type AnsiColor, type AnsiLine, type AnsiSpan } from "./ansi";
import {
  compareTerminalIds,
  nextTerminalId,
  terminalTitle,
} from "./terminalModel";

/** Bounded scrollback — completed lines beyond this are dropped from the head. */
const MAX_SCROLLBACK_LINES = 2000;
/** Output is coalesced into at most one re-render per frame-ish interval. */
const FLUSH_MS = 33;
const FONT_SIZE = typeScale.codeSmall.fontSize as number;
const LINE_HEIGHT = typeScale.codeSmall.lineHeight as number;
/** Geist Mono advance width ≈ 0.6em. Measuring a glyph per layout pass would be exact but
 * needs a hidden text probe on every theme/zoom change; the ratio is within a column. */
const CHAR_WIDTH = FONT_SIZE * 0.6;
const PADDING_H = 14;
const MIN_COLS = 20;
const MIN_ROWS = 4;

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
// Colour resolution
// ---------------------------------------------------------------------------

interface AnsiPalette {
  /** The 16 ANSI slots, in order. */
  basic: string[];
  /** Parsed rgb of each slot, for nearest-match on 256-colour/truecolour output. */
  basicRgb: [number, number, number][];
  fg: string;
  bg: string;
}

function parseHex(hex: string): [number, number, number] {
  const h = hex.replace("#", "");
  return [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
}

/**
 * ANSI's 8+8 palette mapped onto Machined's semantic tokens. Machined has no magenta or cyan
 * ramp — rather than inventing hex outside tokens.ts, magenta reuses the ember ramp and cyan
 * reuses `info`; every other slot has an exact semantic counterpart. Bright variants are the
 * same hue at full ink, normal variants sit one step quieter.
 */
function ansiPalette(tokens: ColorTokens): AnsiPalette {
  const basic = [
    tokens.ink4, // black
    tokens.danger, // red
    tokens.success, // green
    tokens.warn, // yellow
    tokens.info, // blue
    tokens.ember.ember600, // magenta
    tokens.info, // cyan
    tokens.ink2, // white
    tokens.ink3, // bright black
    tokens.danger, // bright red
    tokens.success, // bright green
    tokens.warn, // bright yellow
    tokens.info, // bright blue
    tokens.ember.ember400, // bright magenta
    tokens.info, // bright cyan
    tokens.ink, // bright white
  ];
  return { basic, basicRgb: basic.map(parseHex), fg: tokens.ink, bg: tokens.bg0 };
}

function resolveColor(color: AnsiColor | null, palette: AnsiPalette): string | null {
  if (color == null) return null;
  if (color.kind === "basic") return palette.basic[color.index] ?? palette.fg;
  // Truecolour and the 256-colour cube collapse to the nearest palette entry, so a build
  // script that emits #00d7ff still lands on a token instead of an off-system hex.
  let best = 0;
  let bestDistance = Infinity;
  for (let i = 0; i < palette.basicRgb.length; i += 1) {
    const [r, g, b] = palette.basicRgb[i];
    const distance = (r - color.r) ** 2 + (g - color.g) ** 2 + (b - color.b) ** 2;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = i;
    }
  }
  return palette.basic[best];
}

function spanStyle(span: AnsiSpan, palette: AnsiPalette): TextStyle {
  const { style } = span;
  const fg = resolveColor(style.fg, palette) ?? palette.fg;
  const bg = resolveColor(style.bg, palette);
  const out: TextStyle = { color: style.inverse ? palette.bg : fg };
  if (style.inverse) out.backgroundColor = fg;
  else if (bg) out.backgroundColor = bg;
  if (style.bold) out.fontFamily = monoFamily.medium;
  if (style.dim) out.opacity = 0.6;
  if (style.italic) out.fontStyle = "italic";
  if (style.underline) out.textDecorationLine = "underline";
  return out;
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/** Memoised per line: every output chunk re-renders the list, but only the tail line changed. */
const TerminalLine = React.memo(function TerminalLine({
  line,
  palette,
  caretColor,
}: {
  line: AnsiLine;
  palette: AnsiPalette;
  caretColor: string | null;
}) {
  return (
    <Text style={[styles.line, { color: palette.fg }]} selectable>
      {line.spans.map((span, index) => (
        <Text key={index} style={spanStyle(span, palette)}>
          {span.text}
        </Text>
      ))}
      {caretColor ? <Text style={{ backgroundColor: caretColor, color: caretColor }}> </Text> : null}
    </Text>
  );
});

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
}

type ConnectionStatus = "idle" | "connecting" | "connected" | "disconnected";

export function TerminalDock({
  sessionId,
  terminalId = "term-1",
  compact = false,
  onOpenTerminal,
}: TerminalDockProps) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const palette = useMemo(() => ansiPalette(tokens), [tokens]);

  const bufferRef = useRef(new AnsiScrollback(MAX_SCROLLBACK_LINES));
  const socketRef = useRef<TerminalSocket | null>(null);
  const scrollRef = useRef<ScrollView | null>(null);
  const stickyRef = useRef(true);
  const inputRef = useRef<TextInput | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const restartRef = useRef(false);
  const suppressReconnectRef = useRef(false);
  const terminalStatusRef = useRef<TerminalStatus>("running");

  const [lines, setLines] = useState<AnsiLine[]>([]);
  const [focused, setFocused] = useState(false);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("idle");
  const [terminalStatus, setTerminalStatus] = useState<TerminalStatus>("running");
  const [size, setSize] = useState<{ cols: number; rows: number } | null>(null);
  const [localTerminalId, setLocalTerminalId] = useState(terminalId);
  const [terminals, setTerminals] = useState<TerminalSessionSummary[]>([]);
  const [connectionGeneration, setConnectionGeneration] = useState(0);
  const effectiveTerminalId = compact ? localTerminalId : terminalId;

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
    const cols = Math.max(MIN_COLS, Math.floor((width - PADDING_H * 2) / CHAR_WIDTH));
    const rows = Math.max(MIN_ROWS, Math.floor(height / LINE_HEIGHT));
    setSize((prev) => (prev && prev.cols === cols && prev.rows === rows ? prev : { cols, rows }));
  }, []);

  // One attachment per terminal identity, opened once the pane has been measured so the PTY
  // starts at the right geometry. Closing this socket only detaches; the daemon owns the shell.
  const measured = size != null;
  useEffect(() => {
    if (!baseUrl || !sessionId || !measured) return;
    suppressReconnectRef.current = false;
    const buffer = bufferRef.current;
    buffer.clear();
    setLines(buffer.lines());
    setConnectionStatus("connecting");
    let socket: TerminalSocket;
    let disposed = false;
    const restart = restartRef.current;
    restartRef.current = false;
    try {
      socket = openTerminalSocket(
        baseUrl,
        sessionId,
        {
          onOutput: (chunk) => {
            buffer.write(chunk);
            scheduleFlush();
          },
          onStatus: (nextStatus) => {
            terminalStatusRef.current = nextStatus;
            setTerminalStatus(nextStatus);
            void refreshTerminals();
          },
          onClear: () => {
            buffer.clear();
            setLines(buffer.lines());
          },
          onOpen: () => {
            setConnectionStatus("connected");
            void refreshTerminals();
          },
          onClose: () => {
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
          onError: () => setConnectionStatus("disconnected"),
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
    return () => {
      disposed = true;
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

  useEffect(() => {
    if (size) socketRef.current?.resize(size.cols, size.rows);
  }, [size]);

  const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    stickyRef.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - LINE_HEIGHT;
  }, []);

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
      const native = event.nativeEvent as { key: string; ctrlKey?: boolean };
      const key = native.key;
      if (native.ctrlKey && key.length === 1) {
        const code = key.toUpperCase().charCodeAt(0);
        if (code >= 64 && code <= 95) send(String.fromCharCode(code - 64));
        return;
      }
      // Printable characters arrive through onChangeText — handling them here too would
      // double-send every keystroke.
      if (key.length === 1) return;
      const mapped = CONTROL_KEYS[key];
      if (mapped) send(mapped);
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
    <View style={styles.dock}>
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
              return (
                <Pressable
                  key={id}
                  onPress={() => setLocalTerminalId(id)}
                  accessibilityRole="tab"
                  accessibilityState={{ selected: active }}
                  style={[
                    styles.tab,
                    {
                      borderBottomColor: active ? tokens.accent : "transparent",
                      backgroundColor: active ? hexToRgba(tokens.accent, 0.08) : "transparent",
                    },
                  ]}
                >
                  <Text style={[typeScale.monoMeta, { color: active ? tokens.ink : tokens.ink3 }]}>
                    {terminalTitle(id)}
                    {summary?.status === "exited" ? " · exited" : ""}
                  </Text>
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
      <Pressable style={styles.body} onPress={() => inputRef.current?.focus()} accessibilityRole="none">
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
            />
          ))}
        </ScrollView>
        <TextInput
          ref={inputRef}
          value=""
          onChangeText={(text) => {
            if (text) send(text);
          }}
          onKeyPress={onKeyPress}
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
          {terminalStatus === "exited" ? " · exited" : ""}
          {size ? ` · ${size.cols}×${size.rows}` : ""}
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
  line: { fontFamily: monoFamily.regular, fontSize: FONT_SIZE, lineHeight: LINE_HEIGHT },
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
