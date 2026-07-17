// DESIGN_SYSTEM.md §6 CommandPalette — Raycast-grade: centered 560pt (wide) / full-height
// Sheet (mobile), one SearchField, grouped results (Sessions/Actions/Navigation), keyboard
// nav + selection tick (web/desktop), ⌘K/Ctrl+K open. FEATURES.md §5 (jump to any session,
// run slash commands in the attached session, fleet actions, local nav). BUILD_ORDER T4.2.
//
// The wide/centered variant gets its own Cast+Rise hybrid entrance (fade + 12px rise, 200ms) —
// the compact variant reuses the shared `Sheet` (Anvil) since a full-height mobile sheet is
// exactly what every other sheet in the app already does.
//
// Sending a slash command (`/plan` etc.) when a session is open: the palette is mounted at
// the app root, outside any `SessionProvider`, so it has no handle to that session's live
// socket. Rather than reach into `sessionContext.tsx` (owned by T3.1, risking a merge clash
// with a parallel task), it opens its OWN short-lived `useSessionSocket` connection — the same
// exported hook `lib/ws.ts` already offers, and the same "short-lived WS attach" idea
// FEATURES.md/T4.3's DecisionPeek uses — sends the one prompt once it's open, then tears the
// connection down. A 5s timeout guards against a session that never opens.
import { router, usePathname } from "expo-router";
import {
  Archive,
  BellDot,
  Check,
  Eye,
  Flame,
  History,
  MoonStar,
  Plus,
  Search,
  Settings2,
  SunMedium,
  Wand2,
} from "lucide-react-native";
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
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

import { ApiError } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { haptics } from "../../lib/haptics";
import { useArchiveSession, useSessions } from "../../lib/queries";
import { usePaletteHotkey } from "../../lib/shortcuts";
import { useSessionSocket } from "../../lib/ws";
import { durations, easings } from "../../theme/motion";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space, type StatusDotState } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { DecisionPeek } from "../cards/DecisionPeek";
import { Badge } from "../ds/Badge";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { SearchField } from "../ds/SearchField";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";

import { BUILTIN_COMMANDS, useSkillCommands } from "../../lib/commands";
import { mergeCommandSources } from "../../lib/commandSources";
const TRANSIENT_SEND_TIMEOUT_MS = 5000;
const PANEL_WIDTH = 580;
const ICON_SIZE = 18;
const ICON_STROKE = 1.75;

/** Prototype's mono keycap hint (`esc`, `⌘N`) — bordered, ink4, monoMeta. Web/desktop only;
 * keyboard hints have no meaning on a touch sheet. */
function KeyHint({ label }: { label: string }) {
  const tokens = useTokens();
  return (
    <View style={[styles.keyHint, { borderColor: tokens.border }]}>
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{label}</Text>
    </View>
  );
}

function activeSessionIdFromPathname(pathname: string): string | null {
  const match = /^\/session\/([^/]+)/.exec(pathname);
  return match ? match[1] : null;
}

type PaletteGroupKey = "sessions" | "actions" | "navigation";

interface PaletteItem {
  id: string;
  group: PaletteGroupKey;
  title: string;
  subtitle?: string;
  keywords?: string;
  disabled?: boolean;
  leading?: React.ReactNode;
  trailing?: React.ReactNode;
  /** Set when `trailing` renders its own interactive control (e.g. a trailing IconButton) — see ListRowProps.hasInteractiveTrailing. */
  trailingInteractive?: boolean;
  onSelect: () => void;
}

const GROUP_LABELS: Record<PaletteGroupKey, string> = {
  sessions: "Sessions",
  actions: "Actions",
  navigation: "Go to",
};
const GROUP_ORDER: PaletteGroupKey[] = ["sessions", "actions", "navigation"];

// ---------------------------------------------------------------------------
// PaletteHost / usePalette — open-state provider, mounted once at the app root.
// ---------------------------------------------------------------------------

interface PaletteContextValue {
  isOpen: boolean;
  open: () => void;
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
  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);

  // Native has no keyboard shortcut path (useHotkeys.ts is a no-op) — the palette opens there
  // via a header IconButton that a later task wires through `usePalette().open`.
  usePaletteHotkey(open);

  const value = useMemo<PaletteContextValue>(() => ({ isOpen, open, close }), [isOpen, open, close]);

  return (
    <PaletteContext.Provider value={value}>
      {children}
      <CommandPalette visible={isOpen} onClose={close} />
    </PaletteContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// CommandPalette
// ---------------------------------------------------------------------------

export interface CommandPaletteProps {
  visible: boolean;
  onClose: () => void;
}

export function CommandPalette({ visible, onClose }: CommandPaletteProps) {
  const tokens = useTokens();
  const { scheme, setScheme } = useTheme();
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

  const { data: sessions } = useSessions();
  const archiveSession = useArchiveSession();
  const skillCommands = useSkillCommands();

  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [peekSessionId, setPeekSessionId] = useState<string | null>(null);

  const activeSessionId = useMemo(() => activeSessionIdFromPathname(pathname), [pathname]);

  // Reset search + keyboard selection on every fresh open.
  useEffect(() => {
    if (visible) {
      setQuery("");
      setSelectedIndex(0);
    }
  }, [visible]);

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
      if (!activeSessionId) return;
      close();
      router.push(`/session/${activeSessionId}`);
      setTransient({ sessionId: activeSessionId, text: cmd });
    },
    [activeSessionId, close],
  );

  // -------------------------------------------------------------------------
  // Result groups
  // -------------------------------------------------------------------------

  const sessionItems = useMemo<PaletteItem[]>(
    () =>
      (sessions ?? []).map((s) => {
        const title = s.title || `session ${s.id.slice(0, 8)}`;
        const state: StatusDotState = s.waiting ? "waiting" : s.busy ? "busy" : "idle";
        return {
          id: `session:${s.id}`,
          group: "sessions",
          title,
          subtitle: s.cwd,
          keywords: `${title} ${s.cwd} ${s.model}`,
          leading: <StatusDot state={state} />,
          trailing: s.waiting ? (
            <View style={styles.trailingRow}>
              <Badge label="NEEDS YOU" tone="danger" />
              <IconButton
                icon={<Eye size={16} strokeWidth={ICON_STROKE} color={tokens.ink3} />}
                onPress={() => {
                  close();
                  setPeekSessionId(s.id);
                }}
                accessibilityLabel={`Review decision for ${title}`}
              />
            </View>
          ) : undefined,
          trailingInteractive: s.waiting,
          onSelect: () => {
            close();
            router.push(`/session/${s.id}`);
          },
        };
      }),
    [sessions, tokens, close],
  );

  const actionItems = useMemo<PaletteItem[]>(() => {
    const items: PaletteItem[] = [
      {
        id: "action:new-session",
        group: "actions",
        title: "Forge a session",
        keywords: "new session create start forge task",
        leading: <Plus size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: !isCompact ? <KeyHint label="⌘N" /> : undefined,
        onSelect: () => {
          close();
          router.push("/new-session");
        },
      },
    ];

    if (activeSessionId) {
      items.push({
        id: "action:archive-current",
        group: "actions",
        title: "Archive current session",
        keywords: "archive stop hide current",
        leading: <Archive size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          const id = activeSessionId;
          close();
          archiveSession.mutate(id, {
            onSuccess: () => {
              haptics.allow();
              toast.show("session archived");
            },
            onError: (err) => {
              haptics.mergeConflict();
              toast.show(err instanceof ApiError ? err.message : "archive failed", { tone: "danger" });
            },
          });
        },
      });
    }

    items.push({
      id: "action:theme-toggle",
      group: "actions",
      title: scheme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      keywords: "theme dark light appearance",
      leading:
        scheme === "dark" ? (
          <SunMedium size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />
        ) : (
          <MoonStar size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />
        ),
      onSelect: () => {
        close();
        setScheme(scheme === "dark" ? "light" : "dark");
      },
    });

    const allCommands = mergeCommandSources(BUILTIN_COMMANDS, skillCommands);
    for (const cmd of allCommands) {
      items.push({
        id: `action:cmd:${cmd.name}`,
        group: "actions",
        title: cmd.name,
        subtitle: cmd.description ?? (activeSessionId ? "send to the current session" : "open a session first"),
        keywords: cmd.description ? `${cmd.name} ${cmd.description}` : cmd.name,
        disabled: !activeSessionId,
        leading: <Wand2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => runSlashCommand(cmd.name),
      });
    }

    return items;
  }, [tokens, activeSessionId, scheme, setScheme, close, archiveSession, toast, runSlashCommand, skillCommands, isCompact]);

  const navigationItems = useMemo<PaletteItem[]>(
    () => [
      {
        id: "nav:fleet",
        group: "navigation",
        title: "Fleet",
        keywords: "fleet home sessions live",
        leading: <Flame size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          close();
          router.push("/");
        },
      },
      {
        id: "nav:inbox",
        group: "navigation",
        title: "Inbox",
        keywords: "inbox waiting decisions needs you",
        leading: <BellDot size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          close();
          router.push("/inbox");
        },
      },
      {
        id: "nav:history",
        group: "navigation",
        title: "History",
        keywords: "history past archived",
        leading: <History size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          close();
          router.push("/history");
        },
      },
      {
        id: "nav:settings",
        group: "navigation",
        title: "Settings",
        keywords: "settings preferences servers",
        leading: <Settings2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          close();
          router.push("/settings");
        },
      },
    ],
    [tokens, close],
  );

  const allItems = useMemo(
    () => [...sessionItems, ...actionItems, ...navigationItems],
    [sessionItems, actionItems, navigationItems],
  );

  const q = query.trim().toLowerCase();
  const filteredItems = useMemo(() => {
    if (!q) return allItems;
    return allItems.filter((item) =>
      `${item.title} ${item.subtitle ?? ""} ${item.keywords ?? ""}`.toLowerCase().includes(q),
    );
  }, [allItems, q]);

  const groupedItems = useMemo(
    () => GROUP_ORDER.map((g) => ({ group: g, items: filteredItems.filter((i) => i.group === g) })),
    [filteredItems],
  );

  // Keyboard nav only tracks enabled (selectable) rows.
  const navigableItems = useMemo(() => filteredItems.filter((i) => !i.disabled), [filteredItems]);
  const selectedId = navigableItems[selectedIndex]?.id;

  useEffect(() => {
    setSelectedIndex((index) => Math.min(index, Math.max(0, navigableItems.length - 1)));
  }, [navigableItems.length]);

  // -------------------------------------------------------------------------
  useEffect(() => {
    if (Platform.OS !== "web" || !visible) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        return;
      }
      const target = e.target as HTMLElement | null;
      const isEditing = target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.isContentEditable;
      if (isEditing) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => (navigableItems.length ? (i + 1) % navigableItems.length : 0));
        haptics.select();
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) =>
          navigableItems.length ? (i - 1 + navigableItems.length) % navigableItems.length : 0,
        );
        haptics.select();
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        navigableItems[selectedIndex]?.onSelect();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [visible, navigableItems, selectedIndex, close]);

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
      opacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
      translateY.value = withTiming(0, { duration: durations.base, easing: easings.standard });
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

  const hasResults = filteredItems.length > 0;

  const body = (
    <View style={[styles.body, { paddingTop: Math.max(12, insets.top) }]}>
      <View style={styles.searchWrap}>
        <View style={styles.searchField}>
          <SearchField
            value={query}
            onChangeText={setQuery}
            placeholder="jump to a session, run a command…"
            showCancel={false}
            autoFocus={visible}
            accessibilityLabel="Command palette search"
          />
        </View>
        {!isCompact ? <KeyHint label="esc" /> : null}
      </View>
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={[styles.scrollContent, { paddingBottom: keyboardHeight + insets.bottom + space.space24 }]}
        keyboardShouldPersistTaps="handled"
      >
        {hasResults ? (
          groupedItems.map(({ group, items }) =>
            items.length > 0 ? (
              <View key={group}>
                <SectionHeader>{GROUP_LABELS[group]}</SectionHeader>
                {items.map((item) => (
                  <PaletteRow key={item.id} item={item} selected={item.id === selectedId} />
                ))}
              </View>
            ) : null,
          )
        ) : (
          <EmptyState icon={Search} message="no matches for that search" />
        )}
        {!isCompact && hasResults ? (
          <View style={[styles.footerRow, { borderTopColor: tokens.border }]}>
            <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>↑↓ navigate · ↵ select</Text>
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

  if (isCompact) {
    return (
      <>
        <Sheet visible={visible} onClose={close} maxHeightRatio={1} accessibilityLabel="Command palette">
          {body}
        </Sheet>
        {decisionPeek}
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
            <View style={[styles.centerWrap, { pointerEvents: "box-none" }]}>
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
              >
                {body}
              </Animated.View>
            </View>
          </View>
        </Modal>
      ) : null}
      {decisionPeek}
    </>
  );
}

function PaletteRow({ item, selected }: { item: PaletteItem; selected: boolean }) {
  const tokens = useTokens();
  const showTick = Platform.OS === "web" && selected && !item.disabled;

  const trailing =
    item.trailing || showTick ? (
      <View style={styles.trailingRow}>
        {item.trailing}
        {showTick ? <Check size={16} strokeWidth={2} color={tokens.accent} /> : null}
      </View>
    ) : undefined;

  return (
    <View style={{ backgroundColor: showTick ? tokens.selection : "transparent" }}>
      <ListRow
        title={item.title}
        subtitle={item.subtitle}
        leading={item.leading}
        trailing={trailing}
        onPress={item.disabled ? undefined : item.onSelect}
        disabled={item.disabled}
        accessibilityLabel={item.subtitle ? `${item.title}, ${item.subtitle}` : item.title}
        hasInteractiveTrailing={item.trailingInteractive}
      />
    </View>
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
  trailingRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  centerWrap: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space32 },
  centeredPanel: { overflow: "hidden" },
  keyHint: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: space.space4, paddingVertical: 1 },
  footerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    paddingHorizontal: space.space16,
    paddingTop: space.space12,
    marginTop: space.space8,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
});
