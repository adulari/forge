// DESIGN_SYSTEM.md §6 CommandPalette — Raycast-grade: centered 560pt (wide) / full-height
// Sheet (mobile), one SearchField, category chips (All, Sessions, Commands, Files, Settings),
// capped result sections, keyboard nav + ARIA combobox (web/desktop), ⌘K/Ctrl+K open. ⌘P opens
// the same palette on the Sessions chip. FEATURES.md §5 (jump to any session, run slash commands
// in the attached session, fleet actions, local nav). BUILD_ORDER T4.2.
//
// Where the parts live: ranking and section maths in lib/palette/, local entries in
// usePaletteSources, search-backed entries (message hits, other hosts, files) in
// usePaletteSearch, rows and chips in PaletteRows.
//
// The wide/centered variant gets its own Cast+Rise hybrid entrance (fade + 12px rise, 200ms) —
// the compact variant reuses the shared `Sheet` (Anvil) since a full-height mobile sheet is
// exactly what every other sheet in the app already does.
//
// Sending a slash command (`/plan` etc.) when a session is open: the palette is mounted at
// the app root, outside any `SessionProvider`, so it has no handle to that session's live
// socket. It opens its OWN short-lived `useSessionSocket` connection — the same exported hook
// `lib/ws.ts` already offers — sends the one prompt once it's open, then tears the connection
// down. A 5s timeout guards against a session that never opens.
import { router, usePathname } from "expo-router";
import { Search } from "lucide-react-native";
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  ActivityIndicator,
  BackHandler,
  Keyboard,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Animated, {
  cancelAnimation,
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

import { useAuth } from "../../lib/auth";
import { haptics } from "../../lib/haptics";
import { openSession } from "../../lib/nav";
import { rankItems } from "../../lib/palette/fuzzy";
import { getRecents, loadRecents, rememberRecent, subscribeRecents } from "../../lib/palette/recents";
import {
  FILES_EMPTY_COPY,
  SECTION_LABELS,
  buildSections,
  cycleCategory,
  isImeComposing,
  moreRowId,
  pageIndex,
  selectableRows,
  stepIndex,
  type PaletteCategory,
  type PaletteRow,
  type PaletteSection,
} from "../../lib/palette/sections";
import { usePaletteHotkey, useThreadSearchHotkey } from "../../lib/shortcuts";
import { useCreateSession, usePastSessions } from "../../lib/queries";
import { useOverlayEscape } from "../../lib/useOverlayEscape";
import { useSessionSocket } from "../../lib/ws";
import { durations, easings } from "../../theme/motion";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";
import { formatCwd, formatRelativeTime, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { DecisionPeek } from "../cards/DecisionPeek";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { SearchField } from "../ds/SearchField";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { useActiveSessionId } from "../shell/activeSession";
import { SchedulesPanel } from "../shell/SchedulesPanel";
import { WhatsNewPanel } from "../shell/WhatsNewPanel";
import { CategoryChips, KeyHint, MoreOption, PaletteOption } from "./PaletteRows";
import { optionDomId, PALETTE_LISTBOX_ID, type PaletteItem, type PaletteMode } from "./paletteTypes";
import { usePaletteSearch } from "./usePaletteSearch";
import { usePaletteSources } from "./usePaletteSources";

export type { PaletteMode } from "./paletteTypes";

const TRANSIENT_SEND_TIMEOUT_MS = 5000;
// D/W Palette (docs/design/machined INVENTORY.md): centered panel 620-660px.
const PANEL_WIDTH = 620;
// Rows rendered in the commit that opens the palette (the panel shows ~10 at desktop height).
const FIRST_PAINT_ROWS = 12;
const NO_EXPANSION: ReadonlySet<PaletteSection> = new Set();
// The files header carries a project and branch name, whose case is not ours to change.
const LITERAL_HEADER = { textTransform: "none" } as const;
// Marks the palette's DOM subtree so its own search <input> is not mistaken for "typing elsewhere".
const PALETTE_ROOT_PROPS: object = Platform.OS === "web" ? { dataSet: { paletteRoot: "true" } } : {};

/** D Palette footer breadcrumb (`Fleet · Inbox • · History · Settings`) — a clickable
 * tab-bar equivalent for wide layouts that have no tab bar of their own. */
const BREADCRUMB_ROUTES: { href: "/" | "/inbox" | "/history" | "/settings"; label: string }[] = [
  { href: "/", label: "Fleet" },
  { href: "/inbox", label: "Inbox" },
  { href: "/history", label: "History" },
  { href: "/settings", label: "Settings" },
];

// ---------------------------------------------------------------------------
// PaletteHost / usePalette — open-state provider, mounted once at the app root.
// ---------------------------------------------------------------------------

interface PaletteContextValue {
  isOpen: boolean;
  open: (mode?: PaletteMode) => void;
  close: () => void;
}

const PaletteContext = createContext<PaletteContextValue | null>(null);

export function usePalette(): PaletteContextValue {
  const ctx = useContext(PaletteContext);
  if (!ctx) throw new Error("usePalette must be used within a PaletteHost");
  return ctx;
}

export function PaletteHost({ children }: { children: React.ReactNode }) {
  const [isOpen, setIsOpen] = useState(false);
  const [mode, setMode] = useState<PaletteMode>("default");
  const open = useCallback((m: PaletteMode = "default") => {
    setMode(m);
    setIsOpen(true);
  }, []);
  const close = useCallback(() => setIsOpen(false), []);

  // Native has no keyboard shortcut path (useHotkeys.ts is a no-op) — the palette opens there
  // via a header IconButton that a later task wires through `usePalette().open`.
  usePaletteHotkey(() => open("default"));
  useThreadSearchHotkey(() => open("search"));

  const value = useMemo<PaletteContextValue>(() => ({ isOpen, open, close }), [isOpen, open, close]);

  return (
    <PaletteContext.Provider value={value}>
      {children}
      <CommandPalette visible={isOpen} mode={mode} onClose={close} />
    </PaletteContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

export interface CommandPaletteProps {
  visible: boolean;
  mode?: PaletteMode;
  onClose: () => void;
}

function activeSessionIdFromPathname(pathname: string): string | null {
  const match = /^\/session\/([^/]+)/.exec(pathname);
  return match ? match[1] : null;
}

export function CommandPalette({ visible, mode = "default", onClose }: CommandPaletteProps) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const { isCompact } = useBreakpoint();
  const { width: windowWidth, height: windowHeight } = useWindowDimensions();
  const toast = useToast();
  const pathname = usePathname();
  const { baseUrl } = useAuth();
  const reduced = useReducedMotion();
  const insets = useSafeAreaInsets();
  const [keyboardHeight, setKeyboardHeight] = useState(0);

  useEffect(() => {
    if (Platform.OS === "web") return;
    const showEvent = Platform.OS === "ios" ? "keyboardWillShow" : "keyboardDidShow";
    const hideEvent = Platform.OS === "ios" ? "keyboardWillHide" : "keyboardDidHide";
    const show = Keyboard.addListener(showEvent, (event) => setKeyboardHeight(event.endCoordinates.height));
    const hide = Keyboard.addListener(hideEvent, () => setKeyboardHeight(0));
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);

  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<PaletteCategory>("all");
  const [expanded, setExpanded] = useState<ReadonlySet<PaletteSection>>(NO_EXPANSION);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [peekSessionId, setPeekSessionId] = useState<string | null>(null);
  const [whatsNewVisible, setWhatsNewVisible] = useState(false);
  const [schedulesVisible, setSchedulesVisible] = useState(false);

  const routedSessionId = useMemo(() => activeSessionIdFromPathname(pathname), [pathname]);
  const shellSessionId = useActiveSessionId();
  const recents = useSyncExternalStore(subscribeRecents, getRecents, getRecents);

  // First paint renders only what fits on screen; the rest follows on the next tick. Mounting
  // every row in the open-keystroke's own commit was two thirds of the time to a visible palette.
  const [rowLimit, setRowLimit] = useState(FIRST_PAINT_ROWS);

  // Reset search, category + keyboard selection on every fresh open.
  useEffect(() => {
    if (visible) {
      void loadRecents();
      setQuery("");
      setCategory(mode === "search" ? "sessions" : "all");
      setExpanded(NO_EXPANSION);
      setSelectedIndex(0);
      setRowLimit(FIRST_PAINT_ROWS);
      const timer = setTimeout(() => setRowLimit(Number.POSITIVE_INFINITY), 80);
      return () => clearTimeout(timer);
    }
  }, [visible, mode]);

  const close = onClose;

  // -------------------------------------------------------------------------
  // Slash-command "send as prompt": a short-lived secondary WS connection, only
  // opened while there's a pending command — see file header for why.
  // -------------------------------------------------------------------------
  const [transient, setTransient] = useState<{ sessionId: string; text: string } | null>(null);
  const { send: transientSend, connectionState: transientState } = useSessionSocket(
    baseUrl,
    transient?.sessionId ?? null,
  );
  const transientHandledRef = useRef(false);

  useEffect(() => {
    transientHandledRef.current = false;
  }, [transient]);

  useEffect(() => {
    if (!transient || transientState !== "open" || transientHandledRef.current) return;
    transientHandledRef.current = true;
    transientSend({ kind: "prompt", text: transient.text });
    haptics.sendPrompt();
    toast.show(`sent ${transient.text}`);
    setTransient(null);
  }, [transient, transientState, transientSend, toast]);

  useEffect(() => {
    if (!transient) return;
    const timer = setTimeout(() => {
      if (transientHandledRef.current) return;
      transientHandledRef.current = true;
      toast.show("couldn't reach the session", { tone: "danger" });
      setTransient(null);
    }, TRANSIENT_SEND_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [transient, toast]);

  const runSlashCommand = useCallback(
    (cmd: string) => {
      if (!routedSessionId) return;
      close();
      router.push(`/session/${routedSessionId}`);
      setTransient({ sessionId: routedSessionId, text: cmd });
    },
    [routedSessionId, close],
  );

  // -------------------------------------------------------------------------
  // Entries
  // -------------------------------------------------------------------------
  const openWhatsNew = useCallback(() => setWhatsNewVisible(true), []);
  const openSchedules = useCallback(() => setSchedulesVisible(true), []);
  const { sessions, sessionItems, commandItems, settingsItems } = usePaletteSources({
    close,
    isCompact,
    activeSessionId: routedSessionId,
    recents,
    openWhatsNew,
    openSchedules,
    peekDecision: setPeekSessionId,
    runSlashCommand,
  });

  const q = query.trim();
  const searchSessionsEnabled = visible && (category === "all" || category === "sessions");
  const searchFilesEnabled = visible && (category === "all" || category === "files");
  const search = usePaletteSearch({
    query,
    visible,
    searchSessionsEnabled,
    searchFilesEnabled,
    isCompact,
    activeSessionId: shellSessionId,
    sessions,
    close,
  });

  // Past sessions only join the Sessions list before a search term narrows it; a term goes to the
  // daemon's transcript search instead.
  const createSession = useCreateSession();
  const pastSessionsQuery = usePastSessions();
  const pastItems = useMemo<PaletteItem[]>(() => {
    if (category !== "sessions" || q.length >= 2) return [];
    return (pastSessionsQuery.data?.pages.flat() ?? []).map((p) => {
      const title = p.title || `session ${p.id.slice(0, 8)}`;
      const age = formatRelativeTime(p.last_activity * 1000);
      return {
        id: `past:${p.id}`,
        section: "sessions" as const,
        title,
        subtitle: p.preview ? `${formatCwd(p.cwd)} · ${age} — ${p.preview}` : `${formatCwd(p.cwd)} · ${age}`,
        keywords: p.cwd,
        recency: p.last_activity,
        leading: <StatusDot state="done" />,
        onSelect: () => {
          close();
          createSession.mutate({ resume: p.id }, { onSuccess: (res) => openSession(res.id) });
        },
      };
    });
  }, [category, q, pastSessionsQuery.data, close, createSession]);

  const ranked = useMemo(() => {
    const hitSessionIds = new Set(search.sessionHits.map((hit) => hit.sessionId).filter(Boolean));
    const sessionPool = [
      ...search.sessionHits,
      ...sessionItems.filter((item) => !hitSessionIds.has(item.sessionId)),
      ...pastItems,
    ];
    return rankItems([...sessionPool, ...commandItems, ...search.fileItems, ...settingsItems], q);
  }, [search.sessionHits, search.fileItems, sessionItems, pastItems, commandItems, settingsItems, q]);

  const counts = useMemo(() => {
    const bySection: Partial<Record<PaletteCategory, number>> = {};
    for (const item of ranked) bySection[item.section] = (bySection[item.section] ?? 0) + 1;
    return q ? { ...bySection, all: ranked.length } : {};
  }, [ranked, q]);

  const blocks = useMemo(() => buildSections(ranked, category, expanded), [ranked, category, expanded]);
  const rows = useMemo(() => selectableRows(blocks), [blocks]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [q, category]);
  useEffect(() => {
    setSelectedIndex((index) => Math.min(index, Math.max(0, rows.length - 1)));
  }, [rows.length]);
  const selectedRow = rows[selectedIndex];
  const selectedId = selectedRow?.id;

  const chooseCategory = useCallback((next: PaletteCategory) => {
    setCategory(next);
    setExpanded(NO_EXPANSION);
  }, []);

  const activate = useCallback(
    (row: PaletteRow<PaletteItem> | undefined) => {
      if (!row) return;
      if (row.kind === "more") {
        if (category === "all") chooseCategory(row.section);
        else setExpanded((prev) => new Set(prev).add(row.section));
        return;
      }
      if (row.item.section === "commands" || row.item.section === "settings") rememberRecent(row.item.id);
      row.item.onSelect();
    },
    [category, chooseCategory],
  );

  // -------------------------------------------------------------------------
  // Keyboard (web/desktop). Escape belongs to the overlay stack, not to this handler.
  // -------------------------------------------------------------------------
  useOverlayEscape(visible && !isCompact, close);

  useEffect(() => {
    if (Platform.OS !== "web" || !visible) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (isImeComposing(e)) return;
      if (e.key === "Tab" && e.ctrlKey) {
        e.preventDefault();
        chooseCategory(cycleCategory(category, e.shiftKey ? -1 : 1));
        haptics.select();
        return;
      }
      const target = e.target as HTMLElement | null;
      // The autofocused SearchField is itself an <input> — without this exemption `isEditing`
      // is true the instant the palette opens, and Arrow/Enter never reach the branches below.
      const withinPalette = typeof target?.closest === "function" && target.closest("[data-palette-root]") != null;
      const isEditing =
        !withinPalette &&
        (target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.isContentEditable);
      if (isEditing) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => stepIndex(i, e.key === "ArrowDown" ? 1 : -1, rows.length));
        haptics.select();
      } else if (e.key === "PageDown" || e.key === "PageUp") {
        e.preventDefault();
        setSelectedIndex((i) => pageIndex(i, e.key === "PageDown" ? 1 : -1, rows.length));
        haptics.select();
      } else if (e.key === "Enter") {
        e.preventDefault();
        activate(rows[selectedIndex]);
      }
    };
    // Capture phase: react-native-web's TextInput stops keydown from bubbling past itself, so a
    // bubble listener never sees a key typed in the search field.
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [visible, rows, selectedIndex, category, activate, chooseCategory]);

  useEffect(() => {
    if (Platform.OS !== "web" || !visible || !selectedId) return;
    document.getElementById(optionDomId(selectedId))?.scrollIntoView({ block: "nearest" });
  }, [visible, selectedId]);

  // -------------------------------------------------------------------------
  // Wide/centered variant: custom Cast+Rise hybrid entrance (fade + 12px rise, 200ms).
  // The compact variant below reuses the shared `Sheet` (Anvil) instead.
  // -------------------------------------------------------------------------
  const [mounted, setMounted] = useState(visible);
  const opacity = useSharedValue(0);
  const translateY = useSharedValue(12);
  const scrimOpacity = useSharedValue(0);

  useEffect(() => {
    if (visible) setMounted(true);
  }, [visible]);

  useEffect(() => {
    if (isCompact || !mounted) return;
    if (visible) {
      if (reduced) {
        opacity.value = 1;
        translateY.value = 0;
        scrimOpacity.value = 1;
        return;
      }
      opacity.value = withTiming(1, { duration: durations.fast, easing: easings.standard });
      translateY.value = withTiming(0, { duration: durations.fast, easing: easings.standard });
      scrimOpacity.value = withTiming(1, { duration: durations.fast, easing: easings.standard });
    } else {
      if (reduced) {
        opacity.value = 0;
        translateY.value = 12;
        scrimOpacity.value = 0;
        setMounted(false);
        return;
      }
      opacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit }, (finished) => {
        if (finished) runOnJS(setMounted)(false);
      });
      translateY.value = withTiming(8, { duration: durations.fast, easing: easings.exit });
      scrimOpacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, mounted, reduced, isCompact]);

  useEffect(() => {
    return () => {
      cancelAnimation(opacity);
      cancelAnimation(translateY);
      cancelAnimation(scrimOpacity);
    };
  }, [opacity, translateY, scrimOpacity]);

  const panelStyle = useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }],
  }));
  const scrimStyle = useAnimatedStyle(() => ({ opacity: scrimOpacity.value }));

  // Android hardware back closes the wide/centered modal (the compact Sheet already handles
  // this itself).
  useEffect(() => {
    if (!visible || isCompact || Platform.OS === "web") return;
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      close();
      return true;
    });
    return () => sub.remove();
  }, [visible, isCompact, close]);

  const depth = scheme === "dark" ? depthDark : depthLight;
  const panelWidth = Math.min(PANEL_WIDTH, windowWidth - space.space32 * 2);
  const panelMaxHeight = windowHeight * 0.7;

  const waitingBadge = (sessions ?? []).some((s) => s.waiting);
  const sessionsInView = category === "all" || category === "sessions";
  const filesInView = category === "all" || category === "files";
  const searchPending = (sessionsInView && search.sessionPhase === "searching") || (filesInView && search.filesPhase === "searching");
  const sessionsUnavailable = sessionsInView && (search.sessionPhase === "error" || search.sessionPhase === "indexing");
  const filesBroken = category === "files" && search.filesPhase === "error";

  const comboboxProps =
    Platform.OS === "web"
      ? ({
          role: "combobox",
          "aria-expanded": rows.length > 0,
          "aria-controls": PALETTE_LISTBOX_ID,
          "aria-autocomplete": "list",
          "aria-activedescendant": selectedId ? optionDomId(selectedId) : undefined,
        } as object)
      : {};
  const listboxProps =
    Platform.OS === "web"
      ? ({ nativeID: PALETTE_LISTBOX_ID, role: "listbox", "aria-label": "Results" } as object)
      : {};

  const resultLabel = rows.length === 1 ? "1 result" : `${rows.length} results`;

  const statusLine = sessionsUnavailable ? (
    <View style={styles.statusLine}>
      {search.sessionPhase === "indexing" ? <ActivityIndicator size="small" color={tokens.accent} /> : null}
      <Text style={[typeScale.monoMeta, styles.statusText, { color: tokens.ink3 }]} numberOfLines={1}>
        {search.sessionPhase === "indexing" ? "Indexing session history…" : "Message search unavailable"}
      </Text>
      <Pressable onPress={search.retry} accessibilityRole="button" accessibilityLabel="Retry message search">
        <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>Retry</Text>
      </Pressable>
    </View>
  ) : searchPending && rows.length > 0 ? (
    <View style={styles.statusLine}>
      <ActivityIndicator size="small" color={tokens.accent} />
      <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
        {search.remoteHostsSearching > 0 ? `Searching ${search.remoteHostsSearching + 1} hosts…` : "Searching…"}
      </Text>
    </View>
  ) : null;

  const emptyBody = (() => {
    if (category === "files" && (search.filesEmpty === "no-session" || search.filesEmpty === "short-query")) {
      return <EmptyState icon={Search} message={FILES_EMPTY_COPY[search.filesEmpty]} />;
    }
    if (filesBroken) {
      return (
        <EmptyState
          icon={Search}
          message="Couldn't search file names"
          action={<Button label="Retry" variant="secondary" onPress={search.retry} />}
        />
      );
    }
    if (searchPending) {
      return (
        <View style={styles.searching}>
          <ActivityIndicator color={tokens.accent} />
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            {category === "files" ? "Searching file names…" : "Searching every session…"}
          </Text>
        </View>
      );
    }
    if (sessionsUnavailable) {
      return (
        <EmptyState
          icon={Search}
          message={search.sessionPhase === "indexing" ? "Indexing session history…" : (search.sessionError ?? "Message search unavailable")}
          action={<Button label="Retry" variant="secondary" onPress={search.retry} />}
        />
      );
    }
    return <EmptyState icon={Search} message={category === "files" ? "No file names match" : "no matches for that search"} />;
  })();

  let rowBudget = rowLimit;
  const body = (
    <View style={[styles.body, { paddingTop: Math.max(12, insets.top) }]} {...PALETTE_ROOT_PROPS}>
      <View style={styles.searchWrap}>
        <View style={styles.searchField}>
          <SearchField
            value={query}
            onChangeText={setQuery}
            placeholder={isCompact ? "Search or run a command…" : "jump to a session, run a command, find a file…"}
            showCancel={false}
            autoFocus={visible}
            accessibilityLabel="Command palette search"
            {...comboboxProps}
          />
        </View>
        {!isCompact ? <KeyHint label="esc" /> : null}
      </View>
      <CategoryChips active={category} counts={counts} onSelect={chooseCategory} />
      {statusLine}
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={[styles.scrollContent, { paddingBottom: keyboardHeight + insets.bottom + space.space24 }]}
        keyboardShouldPersistTaps="handled"
      >
        {rows.length === 0 && category === "files" && search.filesEmpty === "idle" ? (
          <SectionHeader style={LITERAL_HEADER}>{search.filesHeader}</SectionHeader>
        ) : null}
        {rows.length > 0 ? (
          <View {...listboxProps}>
            {blocks.map((block) => {
              const shown = block.rows.slice(0, Math.max(0, rowBudget));
              rowBudget -= shown.length;
              if (shown.length === 0) return null;
              const label = block.section === "files" ? search.filesHeader : SECTION_LABELS[block.section];
              return (
                <View
                  key={block.section}
                  {...(Platform.OS === "web" ? ({ role: "group", "aria-label": label } as object) : {})}
                >
                  <SectionHeader style={block.section === "files" ? LITERAL_HEADER : undefined}>{label}</SectionHeader>
                  {shown.map((row) => {
                    const index = rows.findIndex((candidate) => candidate.id === row.id);
                    if (row.kind === "more") {
                      return (
                        <MoreOption
                          key={row.id}
                          id={moreRowId(row.section)}
                          hidden={row.hidden}
                          label={SECTION_LABELS[row.section]}
                          selected={row.id === selectedId}
                          onPress={() => activate(row)}
                          onHover={() => setSelectedIndex(index)}
                        />
                      );
                    }
                    return (
                      <PaletteOption
                        key={row.id}
                        item={row.item}
                        query={q}
                        selected={row.id === selectedId}
                        onActivate={() => activate(row)}
                        onHover={() => index >= 0 && setSelectedIndex(index)}
                      />
                    );
                  })}
                </View>
              );
            })}
          </View>
        ) : (
          emptyBody
        )}
        {!isCompact && rows.length > 0 ? (
          <View style={[styles.footerRow, { borderTopColor: tokens.border }]}>
            <Text
              style={[typeScale.monoMeta, { color: tokens.ink4 }]}
              role={Platform.OS === "web" ? "status" : undefined}
              aria-live={Platform.OS === "web" ? "polite" : undefined}
            >
              {`${q ? `${resultLabel} · ` : ""}↑↓ navigate · ↵ select · ⌃⇥ category`}
            </Text>
            <View style={styles.footerBreadcrumb}>
              {BREADCRUMB_ROUTES.map((r, i) => {
                const active =
                  r.href === "/" ? pathname === "/" || pathname.startsWith("/session") : pathname.startsWith(r.href);
                return (
                  <React.Fragment key={r.href}>
                    {i > 0 ? <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}> · </Text> : null}
                    <Pressable
                      onPress={() => {
                        close();
                        router.push(r.href);
                      }}
                      accessibilityRole="link"
                      accessibilityLabel={r.label}
                    >
                      <Text style={[typeScale.monoMeta, { color: active ? tokens.ink2 : tokens.ink4 }]}>
                        {r.label}
                        {r.href === "/inbox" && waitingBadge ? " •" : ""}
                      </Text>
                    </Pressable>
                  </React.Fragment>
                );
              })}
            </View>
          </View>
        ) : null}
      </ScrollView>
    </View>
  );

  const decisionPeek = (
    <DecisionPeek
      sessionId={peekSessionId}
      visible={peekSessionId != null}
      onClose={() => setPeekSessionId(null)}
    />
  );
  const paletteReachablePanels = (
    <>
      {decisionPeek}
      <WhatsNewPanel visible={whatsNewVisible} onClose={() => setWhatsNewVisible(false)} />
      <SchedulesPanel visible={schedulesVisible} onClose={() => setSchedulesVisible(false)} />
    </>
  );

  if (isCompact) {
    return (
      <>
        <Sheet visible={visible} onClose={close} maxHeightRatio={1} accessibilityLabel="Command palette">
          {body}
        </Sheet>
        {paletteReachablePanels}
      </>
    );
  }

  return (
    <>
      {mounted ? (
        <Modal visible={mounted} transparent animationType="none" onRequestClose={close} statusBarTranslucent>
          <View style={StyleSheet.absoluteFill}>
            <Animated.View style={[StyleSheet.absoluteFill, { backgroundColor: tokens.overlayScrim }, scrimStyle]}>
              <Pressable
                style={StyleSheet.absoluteFill}
                onPress={close}
                accessibilityRole="button"
                accessibilityLabel="Close"
              />
            </Animated.View>
            <View pointerEvents="box-none" style={styles.centerWrap}>
              <Animated.View
                style={[
                  styles.centeredPanel,
                  {
                    backgroundColor: tokens.bg2,
                    borderRadius: radii.radius16,
                    width: panelWidth,
                    maxHeight: panelMaxHeight,
                  },
                  depth.sheet,
                  panelStyle,
                ]}
                accessibilityViewIsModal
                accessibilityLabel="Command palette"
                {...(Platform.OS === "web" ? ({ role: "dialog", "aria-modal": true } as object) : {})}
              >
                {body}
              </Animated.View>
            </View>
          </View>
        </Modal>
      ) : null}
      {paletteReachablePanels}
    </>
  );
}

const styles = StyleSheet.create({
  body: { flex: 1 },
  searchWrap: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingBottom: space.space8,
  },
  searchField: { flex: 1 },
  scroll: { flex: 1 },
  scrollContent: { paddingBottom: space.space24 },
  searching: {
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
    paddingVertical: space.space32,
  },
  statusLine: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingBottom: space.space8,
  },
  statusText: { flexShrink: 1 },
  centerWrap: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space32 },
  centeredPanel: { overflow: "hidden" },
  footerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: space.space16,
    paddingTop: space.space12,
    marginTop: space.space8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  footerBreadcrumb: { flexDirection: "row", alignItems: "center" },
});
