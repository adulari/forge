// Machined desktop shell — two sessions side by side (D Split + Usage, docs/design/machined
// INVENTORY.md L196-260). Each pane carries a 38px mini-header (status dot · title · project ·
// swap / approve / close) and its own composer.
//
// State model, and why pane 0 is not stored: pane 0 IS the routed session. Split mode wraps the
// live `<Stack>` rather than replacing it, so navigation, deep links, the session shell's own
// tabs and every route outside /session keep working untouched — closing the split is a pure
// unwrap. Only the *secondary* pane is state, and it mounts its own `SessionProvider`, giving it
// a real second socket, transcript and composer instead of a read-only mirror.
//
// Desktop-only chrome: `useSplitPanes(enabled)` is passed `isPaired && isExpanded` by the root
// layout, so on compact/medium (phone, narrow web) it reports inactive and nothing here renders.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { router, usePathname } from "expo-router";
import { ArrowLeftRight, Check, SquareSplitHorizontal, X } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, type GestureResponderEvent, type LayoutChangeEvent } from "react-native";

import SessionChat from "../../app/session/[id]/index";
import { useSessions } from "../../lib/queries";
import { SessionProvider, useSessionCtx } from "../../lib/sessionContext";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, space, statusDotColor, type StatusDotState } from "../../theme/tokens";
import { formatCwd, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { routedSessionId, useSessionRow } from "./activeSession";

const SPLIT_STORAGE_KEY = "forge.splitPanes";
const MIN_FRACTION = 0.25;
const MAX_FRACTION = 0.75;
const HEADER_HEIGHT = 38;

interface PersistedSplit {
  on: boolean;
  secondary: string | null;
}

export interface SplitPanesController {
  /** `[primary, secondary]` — pane 0 mirrors the routed session (see the file header). */
  panes: [string | null, string | null];
  /** True only when the split should actually render: enabled surface, on, and a routed session. */
  active: boolean;
  toggle: () => void;
  swap: () => void;
  closePane: (index: 0 | 1) => void;
}

export function useSplitPanes(enabled: boolean): SplitPanesController {
  const pathname = usePathname();
  const primary = routedSessionId(pathname);
  const { data: sessions } = useSessions();
  const [on, setOn] = useState(false);
  const [secondary, setSecondary] = useState<string | null>(null);

  useEffect(() => {
    void AsyncStorage.getItem(SPLIT_STORAGE_KEY).then((raw) => {
      if (!raw) return;
      try {
        const parsed = JSON.parse(raw) as PersistedSplit;
        if (typeof parsed?.on !== "boolean") return;
        setOn(parsed.on);
        setSecondary(typeof parsed.secondary === "string" ? parsed.secondary : null);
      } catch {
        // A corrupt entry just means the shell starts un-split.
      }
    });
  }, []);

  const persist = useCallback((next: PersistedSplit) => {
    void AsyncStorage.setItem(SPLIT_STORAGE_KEY, JSON.stringify(next)).catch(() => undefined);
  }, []);

  const otherSession = useCallback(
    (exclude: string | null) => sessions?.find((row) => row.id !== exclude)?.id ?? null,
    [sessions],
  );

  // The secondary pane must never duplicate pane 0, and must not point at a session the fleet
  // no longer has (archived elsewhere, or a stale persisted id from a previous launch).
  useEffect(() => {
    if (!on || secondary == null || sessions == null) return;
    const stale = secondary === primary || !sessions.some((row) => row.id === secondary);
    if (!stale) return;
    const next = sessions.find((row) => row.id !== primary)?.id ?? null;
    setSecondary(next);
    persist({ on: true, secondary: next });
  }, [on, secondary, primary, sessions, persist]);

  const toggle = useCallback(() => {
    if (on) {
      setOn(false);
      setSecondary(null);
      persist({ on: false, secondary: null });
      return;
    }
    const next = secondary && secondary !== primary ? secondary : otherSession(primary);
    setOn(true);
    setSecondary(next);
    persist({ on: true, secondary: next });
  }, [on, secondary, primary, otherSession, persist]);

  const swap = useCallback(() => {
    if (!secondary || !primary) return;
    setSecondary(primary);
    persist({ on: true, secondary: primary });
    router.push(`/session/${secondary}`);
  }, [secondary, primary, persist]);

  const closePane = useCallback(
    (index: 0 | 1) => {
      // Closing pane 0 promotes the other session rather than dropping both.
      if (index === 0 && secondary) router.push(`/session/${secondary}`);
      setOn(false);
      setSecondary(null);
      persist({ on: false, secondary: null });
    },
    [secondary, persist],
  );

  return useMemo(
    () => ({
      panes: [primary, secondary] as [string | null, string | null],
      active: enabled && on && primary != null,
      toggle,
      swap,
      closePane,
    }),
    [enabled, on, primary, secondary, toggle, swap, closePane],
  );
}

// ---------------------------------------------------------------------------
// Mini-header
// ---------------------------------------------------------------------------

function HeaderAction({
  icon,
  onPress,
  disabled,
  accessibilityLabel,
  accessibilityHint,
}: {
  icon: React.ReactNode;
  onPress: () => void;
  disabled?: boolean;
  accessibilityLabel: string;
  accessibilityHint?: string;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={accessibilityHint}
      style={({ pressed }) => [
        styles.headerAction,
        disabled && styles.headerActionDisabled,
        pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
      ]}
    >
      {icon}
    </Pressable>
  );
}

function PaneHeader({
  title,
  project,
  state,
  canApprove,
  onApprove,
  onSwap,
  onClose,
}: {
  title: string;
  project: string | null;
  state: StatusDotState;
  canApprove: boolean;
  onApprove: () => void;
  onSwap: () => void;
  onClose: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.header, { borderBottomColor: tokens.border }]}>
      <View style={[styles.dot, { backgroundColor: statusDotColor(state, tokens) }]} />
      <Text style={[typeScale.bodyBold, styles.headerTitle, { color: tokens.ink }]} numberOfLines={1}>
        {title}
      </Text>
      {project ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
          {project}
        </Text>
      ) : null}
      <View style={styles.headerSpacer} />
      <HeaderAction
        icon={<ArrowLeftRight size={13} strokeWidth={1.75} color={tokens.ink3} />}
        onPress={onSwap}
        accessibilityLabel="Swap panes"
      />
      <HeaderAction
        icon={<Check size={13} strokeWidth={1.75} color={canApprove ? tokens.accent : tokens.ink4} />}
        onPress={onApprove}
        disabled={!canApprove}
        accessibilityLabel="Approve the pending decision"
      />
      <HeaderAction
        icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />}
        onPress={onClose}
        accessibilityLabel="Close pane"
      />
    </View>
  );
}

/** Pane 0's header, driven off the fleet row (the routed session's own socket lives below,
 * inside the route — reaching it from here would mean opening a second one for the same id). */
function PrimaryPaneHeader({ sessionId, onSwap, onClose }: { sessionId: string; onSwap: () => void; onClose: () => void }) {
  const row = useSessionRow(sessionId);
  const state: StatusDotState = row?.waiting ? "waiting" : row?.busy ? "busy" : "idle";
  return (
    <PaneHeader
      title={row?.title || `session ${sessionId.slice(0, 8)}`}
      project={row ? formatCwd(row.cwd) : null}
      state={state}
      canApprove={!!row?.waiting}
      // The decision itself is answered in the pane below (the permission card is right there)
      // or in Inbox — which is exactly what `desktopMenu`'s own `session:approve` default does
      // when no session claims it.
      onApprove={() => router.push("/inbox")}
      onSwap={onSwap}
      onClose={onClose}
    />
  );
}

// ---------------------------------------------------------------------------
// Secondary pane
// ---------------------------------------------------------------------------

function SecondaryPaneBody({ sessionId, onSwap, onClose }: { sessionId: string; onSwap: () => void; onClose: () => void }) {
  const { snapshot, send } = useSessionCtx();
  const row = useSessionRow(sessionId);
  const pending = snapshot?.permission_prompt != null || snapshot?.question != null;
  const state: StatusDotState = pending ? "waiting" : (snapshot?.busy ?? row?.busy) ? "busy" : "idle";

  return (
    <>
      <PaneHeader
        title={snapshot?.title || row?.title || `session ${sessionId.slice(0, 8)}`}
        project={snapshot ? formatCwd(snapshot.cwd) : row ? formatCwd(row.cwd) : null}
        state={state}
        canApprove={pending && snapshot != null}
        onApprove={() => {
          if (snapshot) send({ kind: "allow", yes: true, seq: snapshot.prompt_seq });
        }}
        onSwap={onSwap}
        onClose={onClose}
      />
      <View style={styles.paneBody}>
        <SessionChat />
      </View>
    </>
  );
}

function SecondaryPane({ sessionId, onSwap, onClose }: { sessionId: string | null; onSwap: () => void; onClose: () => void }) {
  const tokens = useTokens();

  if (!sessionId) {
    return (
      <>
        <View style={[styles.header, { borderBottomColor: tokens.border }]}>
          <Text style={[typeScale.bodyBold, styles.headerTitle, { color: tokens.ink2 }]}>Second pane</Text>
          <View style={styles.headerSpacer} />
          <HeaderAction
            icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />}
            onPress={onClose}
            accessibilityLabel="Close pane"
          />
        </View>
        <View style={styles.paneBody}>
          <EmptyState
            icon={SquareSplitHorizontal}
            message="No second session to show — start another one and it opens here."
            action={<Button label="New session" variant="secondary" onPress={() => router.push("/new-session")} />}
          />
        </View>
      </>
    );
  }

  return (
    // Keyed so switching the secondary session tears the provider (and its socket) down
    // instead of handing a live provider a different id.
    <SessionProvider key={sessionId} sessionId={sessionId}>
      <SecondaryPaneBody sessionId={sessionId} onSwap={onSwap} onClose={onClose} />
    </SessionProvider>
  );
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

export interface SplitPanesProps {
  primaryId: string;
  secondaryId: string | null;
  /** The live route stack — pane 0's body. */
  primary: React.ReactNode;
  onSwap: () => void;
  onClosePane: (index: 0 | 1) => void;
}

export function SplitPanes({ primaryId, secondaryId, primary, onSwap, onClosePane }: SplitPanesProps) {
  const tokens = useTokens();
  const [fraction, setFraction] = useState(0.5);
  // Same shape as DockHost's resizer: gesture bookkeeping is written and read only inside
  // handlers, and `fraction` is the render-visible mirror.
  const drag = useRef({ startX: 0, startFraction: 0.5, fraction: 0.5, width: 0 });

  const onLayout = useCallback((event: LayoutChangeEvent) => {
    drag.current.width = event.nativeEvent.layout.width;
  }, []);

  const onGrantSplit = useCallback((event: GestureResponderEvent) => {
    drag.current.startX = event.nativeEvent.pageX;
    drag.current.startFraction = drag.current.fraction;
  }, []);

  const onMoveSplit = useCallback((event: GestureResponderEvent) => {
    if (drag.current.width <= 0) return;
    const delta = (event.nativeEvent.pageX - drag.current.startX) / drag.current.width;
    const next = Math.max(MIN_FRACTION, Math.min(MAX_FRACTION, drag.current.startFraction + delta));
    drag.current.fraction = next;
    setFraction(next);
  }, []);

  return (
    <View style={styles.row} onLayout={onLayout}>
      <View style={[styles.pane, { flex: fraction }]}>
        <PrimaryPaneHeader sessionId={primaryId} onSwap={onSwap} onClose={() => onClosePane(0)} />
        <View style={styles.paneBody}>{primary}</View>
      </View>
      <View
        onStartShouldSetResponder={() => true}
        onMoveShouldSetResponder={() => true}
        onResponderGrant={onGrantSplit}
        onResponderMove={onMoveSplit}
        style={[styles.splitter, { backgroundColor: tokens.border }]}
        accessibilityRole="adjustable"
        accessibilityLabel="Resize split panes"
      />
      <View style={[styles.pane, { flex: 1 - fraction }]}>
        <SecondaryPane sessionId={secondaryId} onSwap={onSwap} onClose={() => onClosePane(1)} />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flex: 1, flexDirection: "row" },
  pane: { minWidth: 0, flexDirection: "column" },
  paneBody: { flex: 1, minHeight: 0 },
  splitter: { width: StyleSheet.hairlineWidth, marginHorizontal: 2 },
  header: {
    height: HEADER_HEIGHT,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
  },
  headerTitle: { flexShrink: 1 },
  headerSpacer: { flex: 1 },
  headerAction: { width: 26, height: 26, borderRadius: 3, alignItems: "center", justifyContent: "center" },
  headerActionDisabled: { opacity: 0.5 },
  dot: { width: 6, height: 6, borderRadius: 3 },
});
