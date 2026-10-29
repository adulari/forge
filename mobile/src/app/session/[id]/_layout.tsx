// Session shell (T3.1) — mounts SessionProvider ONCE (one `useSessionSocket` for this id)
// and renders header/status-strip/Segmented/banners around a <Slot/> for the active
// segment (Chat/Tasks/Agents/Review). Segment switches use `router.replace` — never a
// remount of this layout — so the socket survives every tab change. See
// mobile/redesign/ARCHITECTURE.md §3 (protocol/prompt_seq), §4.1.4 (history invalidation),
// FEATURES.md §1.2 (Snapshot field -> UI map), DESIGN_SYSTEM.md §6, BUILD_ORDER.md T3.1.
//
// HANDOFF(T3.1 -> T3.2/T3.3/T3.4): this shell's outer SafeAreaView only consumes the
// top/left/right insets (for the header). Segment screens (index/tasks/agents/review.tsx)
// render inside a plain flex-1 View with no safe-area or gutter applied by the shell — each
// segment should use its own `<Screen edges={["left", "right", "bottom"]}>` (omit "top") so
// the bottom home-indicator inset is still respected without double-padding under the header.
import * as Clipboard from "expo-clipboard";
import { router, Slot, useLocalSearchParams, usePathname } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
import React, { Activity, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Banner } from "../../../components/ds/Banner";
import { IconButton } from "../../../components/ds/IconButton";
import { TabStrip, type TabStripOption } from "../../../components/ds/TabStrip";
import { useToast } from "../../../components/ds/ToastHost";
import { OverlayHost } from "../../../components/overlay/OverlayHost";
import { usePalette } from "../../../components/overlay/CommandPalette";
import { ChildSessionHeader } from "../../../components/session/ChildSessionHeader";
import { SessionHeader } from "../../../components/session/SessionHeader";
import {
  KEEP_SEGMENTS_ALIVE,
  RouteScreensHostedContext,
  SegmentKeepAlive,
  SegmentsHostedContext,
  type KeptSegment,
  type SegmentScreen,
} from "../../../components/session/SegmentKeepAlive";
import { SessionLifecycleSheet } from "../../../components/session/SessionLifecycleSheet";
import { useWorkbench } from "../../../components/workbench/WorkbenchProvider";
import { activeWorkbenchSurface } from "../../../components/workbench/model";
import { GoalBanner } from "../../../components/session/GoalBanner";
import { StatusDot } from "../../../components/ds/StatusDot";
import { DuelSheet } from "../../../components/session/DuelSheet";
import { PlanSheet } from "../../../components/session/PlanSheet";

import { ForkSheet } from "../../../components/session/ForkSheet";

import { InitProjectSheet } from "../../../components/session/InitProjectSheet";

import { AssaySheet } from "../../../components/session/AssaySheet";

import { SelfMcpSheet } from "../../../components/session/SelfMcpSheet";


import { CheckpointSheet } from "../../../components/session/CheckpointSheet";


import { PullRequestSheet } from "../../../components/session/PullRequestSheet";


import { MemorySheet } from "../../../components/session/MemorySheet";


import { LatticeSheet } from "../../../components/session/LatticeSheet";
import { StatusStrip } from "../../../components/session/StatusStrip";
import { useAnywhere } from "../../../lib/anywhere/store";
import { useAuth } from "../../../lib/auth";
import { useDesktopMenuAction } from "../../../lib/desktopMenu";
import { assessCompatibility, mismatchMessage } from "../../../lib/diagnostics";
import { socketChipModel } from "../../../lib/socketChip";
import { useHostRestarting } from "../../../lib/hostStatus";
import { goBackOr, openSession } from "../../../lib/nav";
import { useAppShortcut } from "../../../lib/shortcuts";
import { useHistory, usePrefetchSessionPanels, useSessions, useSessionWeeklyDelta, useTurnCompleted } from "../../../lib/queries";
import { useReviewComments } from "../../../lib/reviewComments";
import { SessionProvider, useSessionCtx } from "../../../lib/sessionContext";
import {
  getSessionPrewarmRequest,
  MAX_KEPT_SESSIONS,
  rememberSessionPath,
  settleActivation,
  subscribeSessionPrewarm,
  useSessionActivationOverride,
  useKeepSessionsMounted,
} from "../../../lib/sessionPrewarm";
import { isSegmentPath, type SegmentValue } from "../../../lib/sessionSegments";
import { useSessionSegment } from "../../../lib/useSessionSegment";
import { sessionDisplayTitle } from "../../../lib/sessionTitle";
import { PROTOCOL_VERSION } from "../../../lib/ws";
import { durations, easings } from "../../../theme/motion";
import { useTokens } from "../../../theme/ThemeProvider";
import { radii, space, type StatusDotState } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";
import { routedSessionId } from "../../../components/shell/activeSession";
import { useBreakpoint } from "../../../theme/useBreakpoint";
import { SessionAgents } from "./agents";
import { SessionFiles } from "./files";
import { SessionChat } from "./index";
import { SessionReplayScreen } from "./replay";
import { Review as SessionReview } from "./review";
import { SessionTasks } from "./tasks";
import { SessionTerminal } from "./terminal";
import { WorkflowRunScreen } from "./workflow";
import { WorkflowLibraryScreen } from "./workflows";

// Module-level so the memoised host below never re-renders with the layout's per-frame churn.
const SEGMENT_SCREENS: readonly SegmentScreen[] = [
  { segment: "chat", Component: SessionChat },
  { segment: "tasks", Component: SessionTasks },
  { segment: "agents", Component: SessionAgents },
  { segment: "review", Component: SessionReview },
  { segment: "files", Component: SessionFiles },
  { segment: "replay", Component: SessionReplayScreen },
];

// Sheets mount the first time they open and stay mounted afterwards (so their close animation
// still plays). A closed Sheet still runs window-dimension, safe-area, reanimated and gesture
// hooks on every render of the shell, and the shell renders on every WS frame and keystroke:
// twelve of them cost more than the rest of the header put together.
function OnFirstOpen({ visible, children }: { visible: boolean; children: React.ReactNode }) {
  const [opened, setOpened] = useState(visible);
  if (visible && !opened) setOpened(true);
  return opened ? <>{children}</> : null;
}

function SessionShell({ sessionId, active, hostRoutes }: { sessionId: string; active: boolean; hostRoutes: boolean }) {
  const tokens = useTokens();
  const toast = useToast();
  // A hidden (kept-alive) session must not follow the URL of the session that is open now: it
  // holds the pathname it last had while active, and resumes following when shown again.
  const livePathname = usePathname();
  const [pathname, setPathname] = useState(livePathname);
  // Only follow the URL while it is this session's: right after a rail click the URL still names
  // the previous session for a moment.
  const ownsUrl = routedSessionId(livePathname) === sessionId;
  if (active && ownsUrl && pathname !== livePathname) setPathname(livePathname);
  useEffect(() => {
    if (active && ownsUrl) rememberSessionPath(sessionId, livePathname);
  }, [active, ownsUrl, sessionId, livePathname]);
  const { isCompact, isExpanded } = useBreakpoint();
  const workbench = useWorkbench();
  const activeRightSurface = activeWorkbenchSurface(workbench.state, "right");
  const activeBottomSurface = activeWorkbenchSurface(workbench.state, "bottom");
  const { snapshot, connectionState, send, setHeaderHeight, baseUrl, focusComposer } = useSessionCtx();
  const reviewComments = useReviewComments(sessionId);
  const [duelVisible, setDuelVisible] = useState(false);
  const [planVisible, setPlanVisible] = useState(false);

  const [forkVisible, setForkVisible] = useState(false);

  const [initVisible, setInitVisible] = useState(false);
  const [projectWarningDismissed, setProjectWarningDismissed] = useState(false);
  const [projectSetupRequested, setProjectSetupRequested] = useState(false);

  useEffect(() => {
    setProjectWarningDismissed(false);
    setProjectSetupRequested(false);
  }, [sessionId]);

  const [assayVisible, setAssayVisible] = useState(false);

  const [selfMcpVisible, setSelfMcpVisible] = useState(false);


  const [checkpointVisible, setCheckpointVisible] = useState(false);


  const [pullRequestVisible, setPullRequestVisible] = useState(false);


  const [memoryVisible, setMemoryVisible] = useState(false);


  const [latticeVisible, setLatticeVisible] = useState(false);
  const [lifecycleVisible, setLifecycleVisible] = useState(false);

  // Forge Anywhere — session-surface features (host·transport meta, handoff, share-replay).
  // Gated on `signedIn` so a session opened without an Anywhere account renders identically
  // to before this landed: no host prefix on the meta line, no extra kebab rows.
  const { signedIn } = useAnywhere();
  const { host: authHost } = useAuth();

  // Same query (and cache entry) the session screen reads: one relay request, not two.
  const { data: sessionHistory } = useHistory(sessionId, { includeTools: true });
  const weekly = useSessionWeeklyDelta(sessionId);
  const latestAssistantModel = useMemo(
    () => (sessionHistory?.pages ?? []).flat().find((row) => row.role === "assistant" && row.model)?.model ?? null,
    [sessionHistory],
  );
  const snapshotTitle = snapshot?.title;
  const title = useMemo(
    () => sessionDisplayTitle(snapshotTitle, (sessionHistory?.pages ?? []).flat(), sessionId),
    [snapshotTitle, sessionHistory, sessionId],
  );
  const { open: openPalette } = usePalette();
  const sendWithFeedback = useCallback((input: Parameters<typeof send>[0]) => {
    if (send(input)) return true;
    toast.show("not sent — reconnect and try again", { tone: "danger" });
    return false;
  }, [send, toast]);

  // ARCHITECTURE §4.1.4: on the `busy` true->false edge, invalidate this session's history
  // query so the finalized turn appears from the store. The shell only needs to call the
  // hook to fire that side effect — segments consume useHistory independently.
  useTurnCompleted(snapshot);
  usePrefetchSessionPanels(sessionId, KEEP_SEGMENTS_ALIVE);

  const lastCopyText = useRef<string | null>(null);

  // Settings' removeServer/setActive change `useAuth().baseUrl` reactively, which
  // `SessionProvider` reconnects `useSessionSocket` against immediately — silently pointing
  // this session id at a server it never belonged to. Latch the first server this session was
  // opened against; if the live one ever diverges (including going null), the session is no
  // longer valid here and we bail out instead of limping along against the wrong daemon.
  const ownedBaseUrl = useRef<string | null>(null);
  const leftRef = useRef(false);

  useEffect(() => {
    ownedBaseUrl.current = null;
    leftRef.current = false;
  }, [sessionId]);

  useEffect(() => {
    if (leftRef.current) return;
    if (ownedBaseUrl.current == null) {
      if (baseUrl != null) ownedBaseUrl.current = baseUrl;
      return;
    }
    if (baseUrl !== ownedBaseUrl.current) {
      leftRef.current = true;
      toast.show("server changed — leaving this session", { tone: "danger" });
      router.replace("/(tabs)");
    }
  }, [baseUrl, toast]);

  // copy_text: on change to a new non-null value, set the device clipboard + toast.
  useEffect(() => {
    const text = snapshot?.copy_text ?? null;
    if (text != null && text !== lastCopyText.current) {
      lastCopyText.current = text;
      Clipboard.setStringAsync(text).catch(() => {
        // best-effort — nothing actionable if the clipboard write fails
      });
      toast.show("response copied");
    } else if (text == null) {
      lastCopyText.current = null;
    }
  }, [snapshot?.copy_text, toast]);

  // Reset copy watermark when the session id changes (new socket, fresh snapshot history).
  useEffect(() => {
    lastCopyText.current = null;
  }, [sessionId]);

  const { segment: activeSegment, setSegment: onSegmentChange } = useSessionSegment(pathname, sessionId, KEEP_SEGMENTS_ALIVE);
  // The terminal segment is deliberately not kept alive (hiding it would pause its PTY socket):
  // it mounts when active and unmounts when left.
  const onSegmentPath = isSegmentPath(pathname);
  const keptSegment: KeptSegment | null = onSegmentPath && activeSegment !== "terminal" ? activeSegment : null;
  const terminalActive = KEEP_SEGMENTS_ALIVE && onSegmentPath && activeSegment === "terminal";

  const parentSession = snapshot?.parent ?? null;
  const isChild = parentSession != null;
  const segmentOptions = useMemo<TabStripOption<SegmentValue>[]>(() => {
    const taskCount = snapshot?.tasks.length ?? 0;
    const agentCount = snapshot?.subagents.length ?? 0;
    const reviewPending =
      snapshot?.plan != null || snapshot?.diff != null || reviewComments.length > 0;
    if (isChild) {
      return [
        { value: "chat", label: "Chat" },
        { value: "replay", label: "Replay" },
      ];
    }
    return [
      { value: "chat", label: "Chat" },
      { value: "tasks", label: "Tasks", badge: taskCount || undefined },
      { value: "agents", label: "Agents", badge: agentCount || undefined },
      { value: "review", label: "Review", dot: reviewPending },
      { value: "files", label: "Files" },
      { value: "terminal", label: "Terminal" },
      { value: "replay", label: "Replay" },
    ];
  }, [
    isChild,
    reviewComments.length,
    snapshot?.tasks.length,
    snapshot?.subagents.length,
    snapshot?.plan,
    snapshot?.diff,
  ]);

  const reduced = useReducedMotion();
  const segmentOpacity = useSharedValue(1);
  const segmentY = useSharedValue(0);
  useEffect(() => {
    if (reduced || KEEP_SEGMENTS_ALIVE) { segmentOpacity.value = 1; segmentY.value = 0; return; }
    segmentOpacity.value = 0;
    segmentY.value = 6;
    segmentOpacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
    segmentY.value = withTiming(0, { duration: durations.base, easing: easings.standard });
  }, [activeSegment, reduced, segmentOpacity, segmentY]);
  const segmentStyle = useAnimatedStyle(() => ({ opacity: segmentOpacity.value, transform: [{ translateY: segmentY.value }] }));

  // Web/desktop in-session keyboard shortcuts (native `useHotkey` is a no-op). Alt+C/T/A/R
  // switch the Chat/Tasks/Agents/Review segment (Alt+1..4 is already tab-level navigation in
  // useGlobalShortcuts; ⌘1..9 is a browser-chrome tab switcher that never reaches page JS, so
  // letter keys with Alt are used instead); ⌘E focuses the composer; ⌘. interrupts a busy
  // turn. All use the existing `useHotkey` registry (T4.2/T5.1) — no new listener.
  const interrupt = useCallback(() => {
    // Mirror the Composer Stop button: don't silently drop a Stop sent while the socket is down.
    if (snapshot?.busy && !send({ kind: "interrupt" })) {
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  }, [snapshot?.busy, send, toast]);
  useAppShortcut("session.chat", () => onSegmentChange("chat"));
  useAppShortcut("session.tasks", () => onSegmentChange("tasks"));
  useAppShortcut("session.agents", () => onSegmentChange("agents"));
  useAppShortcut("session.review", () => onSegmentChange("review"));
  useAppShortcut("session.focusComposer", focusComposer);
  useAppShortcut("session.interrupt", interrupt);
  useDesktopMenuAction("session:interrupt", interrupt);

  const closed = snapshot?.closed ?? false;
  const compatibility = snapshot == null
    ? null
    : assessCompatibility(snapshot.protocol, undefined, PROTOCOL_VERSION, "");
  const protocolWarning = compatibility?.status === "client-limited"
    ? compatibility.detail
    : compatibility?.status === "client-outdated" || compatibility?.status === "daemon-outdated"
      ? mismatchMessage(compatibility.status, snapshot?.protocol, PROTOCOL_VERSION)
      : null;
  const publicExposure = (snapshot?.exposure ?? "").startsWith("public");
  const reconnecting = connectionState === "reconnecting";
  const hostRestarting = useHostRestarting();
  // "Unreachable" is only honest when the daemon itself is gone. If the fleet list still
  // answers but no longer contains this session, the session ENDED (archived/discarded) —
  // scary "can't reach forge serve" for a normally-ended session reads as an outage. The
  // check applies from the first reconnect attempt so the ended state lands immediately
  // instead of after the full unreachable backoff.
  const { data: fleetRows } = useSessions();
  const wsDown = connectionState === "reconnecting" || connectionState === "unreachable";
  const sessionEnded = !isChild && wsDown && fleetRows != null && !fleetRows.some((row) => row.id === sessionId);
  const unreachable = connectionState === "unreachable" && !sessionEnded;
  const socketChip = useMemo(() => socketChipModel(connectionState, sessionEnded), [connectionState, sessionEnded]);
  const projectNeedsInitialization = snapshot?.project_initialized === false && !projectWarningDismissed;

  const statusState: StatusDotState =
    snapshot == null
      ? "idle"
      : snapshot.permission_prompt != null || snapshot.question != null
        ? "waiting"
        : snapshot.busy
          ? "busy"
          : snapshot.done || closed
            ? "done"
            : "idle";

  // SessionShell re-renders on every WS snapshot frame (it reads `snapshot` directly for the
  // header/status-strip fields below), so SessionHeader/StatusStrip's own `React.memo` only
  // pays off if the handlers and objects passed to them keep a stable identity across those
  // frames — hoisted here instead of the inline arrow functions/object literals JSX had
  // before, none of which changes what any handler does.
  const onBack = useCallback(() => goBackOr("/(tabs)"), []);
  const parentId = parentSession?.id ?? null;
  // `navigate`, not `back`: a window narrowed from the desktop layout has no stack to pop (the
  // rail replaces sessions), and Back from there landed on the fleet instead of the parent.
  const onBackToParent = useCallback(() => router.navigate(`/session/${parentId}/agents` as never), [parentId]);
  const onOpenParent = useCallback(() => {
    if (parentId == null) return;
    if (isCompact) router.navigate(`/session/${parentId}` as never);
    else openSession(parentId);
  }, [parentId, isCompact]);
  const onNewHere = useCallback(() => {
    router.push({ pathname: "/new-session", params: { cwd: snapshot?.cwd ?? "" } });
  }, [snapshot?.cwd]);
  const onDuel = useCallback(() => setDuelVisible(true), []);
  const onSessionReplay = useCallback(() => router.push(`/session/${sessionId}/replay`), [sessionId]);
  const onPlan = useCallback(() => setPlanVisible(true), []);
  const onWorkflows = useCallback(() => router.push(`/session/${sessionId}/workflows`), [sessionId]);
  const onFork = useCallback(() => setForkVisible(true), []);
  const onInit = useCallback(() => setInitVisible(true), []);
  const onAssay = useCallback(() => setAssayVisible(true), []);
  const onSelfMcp = useCallback(() => setSelfMcpVisible(true), []);
  const onCheckpoint = useCallback(() => setCheckpointVisible(true), []);
  const onPullRequest = useCallback(() => setPullRequestVisible(true), []);
  const onMemory = useCallback(() => setMemoryVisible(true), []);
  const onLattice = useCallback(() => setLatticeVisible(true), []);
  const onManageLifecycle = useCallback(() => setLifecycleVisible(true), []);
  const onToggleFiles = useCallback(() => {
    if (activeRightSurface?.kind === "files") workbench.hidePlacement("right");
    else workbench.openSurface({ kind: "files" });
  }, [activeRightSurface, workbench]);
  const onTogglePreview = useCallback(
    () => workbench.toggleSurface({ kind: "preview", sessionId }),
    [workbench, sessionId],
  );
  const onToggleGitReview = useCallback(() => workbench.toggleSurface({ kind: "git" }), [workbench]);
  const onToggleTerminal = useCallback(() => workbench.toggleSurface({ kind: "terminal" }), [workbench]);
  const openHandoff = useCallback(() => router.push("/anywhere/handoff"), []);
  // `onHandoff` stays `undefined` (not a fresh no-op arrow) when signed out, so SessionHeader's
  // memo sees a stable `undefined` across frames too.
  const onHandoff = signedIn ? openHandoff : undefined;
  // Not offered: the share sheet and the handoff sheet this menu used to open were backed by the
  // in-memory mock client — "Create share link" produced a link to nothing, and the handoff
  // preflight listed files (".env", "assets/demo.mov") no session has. Handoff now opens the real
  // capsule screen; share links are created on the host (`forge anywhere share`). "Switch
  // transport" only ever showed a "lands with the relay backend" toast.
  const onShareReplay: (() => void) | undefined = undefined;
  const onSwitchTransport: (() => void) | undefined = undefined;

  // StatusStrip's `weekly`/`transport` props are object literals — recreating them every
  // render (as the JSX used to do inline) would defeat its memo on every frame even though
  // the underlying values rarely change (weekly usage polls independently of the WS snapshot).
  const weeklySubscription = weekly.mode === "subscription";
  const weeklyProvider = weeklySubscription ? weekly.provider : null;
  const weeklyDeltaPct = weeklySubscription ? weekly.deltaPct : null;
  const weeklyProp = useMemo(
    () => (weeklySubscription ? { provider: weeklyProvider as string, deltaPct: weeklyDeltaPct as number } : null),
    [weeklySubscription, weeklyProvider, weeklyDeltaPct],
  );
  const transportProp = useMemo(
    () =>
      signedIn
        ? {
            hostName: authHost ?? "this host",
            // An Anywhere pairing's base URL is `fany://<host id>`; everything else is a direct URL.
            transport: baseUrl?.startsWith("fany://") ? ("anywhere" as const) : ("direct" as const),
          }
        : undefined,
    [signedIn, authHost, baseUrl],
  );

  const gutter = { paddingHorizontal: isCompact ? space.space16 : space.space24 };
  // DESIGN_SYSTEM.md §7 expanded: the full spec calls for session/[id] to render inline as the
  // right pane of a persistent Fleet rail (master-detail), but that's a routing-architecture
  // change (session/[id] is a sibling Stack route outside (tabs) on purpose — see the HANDOFF
  // in (tabs)/_layout.tsx) out of this fix's bounded scope. Until that lands, the expanded
  // breakpoint fills the full viewport width like every other screen — only `gutter` (space24)
  // keeps header/status/segmented/chat content off the screen edges, no extra centered cap.

  return (
    <View style={[styles.flex, { backgroundColor: tokens.bg1 }]}>
      <SafeAreaView
        edges={["top", "left", "right"]}
        style={{ backgroundColor: tokens.bg2, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: tokens.border }}
        onLayout={(e) => setHeaderHeight(e.nativeEvent.layout.height)}
      >
        {/* Explicit bg2 wrapper fills every header row while SafeAreaView owns the
            physical top inset and reports the complete header height. */}
        <View style={{ backgroundColor: tokens.bg2 }}>
          <View style={gutter}>
          {parentSession != null ? (
            <ChildSessionHeader
              title={title}
              parentTitle={parentSession.title}
              state={statusState}
              model={snapshot?.model && snapshot.model !== "—" ? snapshot.model : latestAssistantModel}
              costUsd={snapshot?.cost_usd ?? 0}
              showBack={!isExpanded}
              onBack={onBackToParent}
              onOpenParent={onOpenParent}
            />
          ) : (
            <>
          <SessionHeader
            title={title}
            state={statusState}
            exposure={snapshot?.exposure ?? "loopback"}
            socketChip={socketChip}
            onBack={onBack}
            showBack={!isExpanded}
            onNewHere={onNewHere}
            onPalette={openPalette}
            onDuel={onDuel}
            onReplay={onSessionReplay}
            onPlan={onPlan}
            onWorkflows={onWorkflows}
            onFork={onFork}
            onInit={onInit}
            onAssay={onAssay}
            onSelfMcp={onSelfMcp}
            onCheckpoint={onCheckpoint}
            onPullRequest={onPullRequest}
            onMemory={onMemory}
            onLattice={onLattice}
            onManageLifecycle={onManageLifecycle}
            onToggleFiles={onToggleFiles}
            onTogglePreview={onTogglePreview}
            onToggleGitReview={onToggleGitReview}
            onToggleTerminal={onToggleTerminal}
            filesActive={activeRightSurface?.kind === "files"}
            previewActive={activeRightSurface?.kind === "preview"}
            gitReviewActive={activeRightSurface?.kind === "git"}
            terminalActive={activeBottomSurface?.kind === "terminal"}
            onHandoff={onHandoff}
            onShareReplay={onShareReplay}
            // Direct<->Anywhere switching needs the relay backend (not built yet) to actually
            // move a live session between transports — surface intent now, land the real flow later.
            onSwitchTransport={onSwitchTransport}
            handoffEligibility="eligible · at checkpoint"
            transportMeta="anywhere → direct"
          />
          <StatusStrip
            state={statusState}
            tier={snapshot?.tier ?? null}
            model={snapshot?.model && snapshot.model !== "—" ? snapshot.model : latestAssistantModel ?? "—"}
            temper={snapshot?.temper ?? "—"}
            effort={snapshot?.effort}
            send={send}
            costUsd={snapshot?.cost_usd ?? 0}
            contextTokens={snapshot?.context_tokens ?? 0}
            contextLimit={snapshot?.context_limit ?? null}
            weekly={weeklyProp}
            cwd={snapshot?.cwd ?? sessionId}
            worktree={snapshot?.worktree ?? null}
            reconnecting={reconnecting && !sessionEnded}
            hostRestarting={hostRestarting}
            // Real sessions run Direct today — there's no relay-hosted session state yet, so
            // `strip` stays unset rather than fabricating one of the 7 Anywhere conditions.
            // A future relay-hosted session sets `transport: "anywhere"` and a real `strip`.
            transport={transportProp}
          />
            </>
          )}
          {snapshot?.workflow ? (
            <Pressable
              onPress={() => router.push(`/session/${sessionId}/workflow`)}
              accessibilityRole="button"
              accessibilityLabel={`${snapshot.workflow.active ? "Live" : "Finished"} workflow ${snapshot.workflow.name ?? "run"}`}
              style={{ flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space8, marginBottom: space.space8, minHeight: 40, paddingHorizontal: space.space12, borderRadius: radii.radiusPill, borderWidth: StyleSheet.hairlineWidth, borderColor: snapshot.workflow.active ? tokens.accent : tokens.border, backgroundColor: snapshot.workflow.active ? tokens.selection : tokens.bg2 }}
            >
              <StatusDot state={snapshot.workflow.active ? "busy" : "done"} />
              <Text style={[typeScale.monoMeta, { flex: 1, color: tokens.ink2 }]} numberOfLines={1}>{`${snapshot.workflow.name ?? "workflow"} · ${snapshot.workflow.active ? "running" : "finished"}`}</Text>
              <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>{snapshot.workflow.active ? "view run" : "view result"}</Text>
            </Pressable>
          ) : null}
          </View>

        <OnFirstOpen visible={duelVisible}><DuelSheet visible={duelVisible} onClose={() => setDuelVisible(false)} send={sendWithFeedback} /></OnFirstOpen>
        <OnFirstOpen visible={planVisible}><PlanSheet
          visible={planVisible}
          onClose={() => setPlanVisible(false)}
          send={sendWithFeedback}
          plan={snapshot?.plan}
        /></OnFirstOpen>

        <OnFirstOpen visible={forkVisible}><ForkSheet visible={forkVisible} onClose={() => setForkVisible(false)} sessionId={sessionId} /></OnFirstOpen>

        <OnFirstOpen visible={initVisible}><InitProjectSheet visible={initVisible} onClose={() => setInitVisible(false)} send={sendWithFeedback} /></OnFirstOpen>

        <OnFirstOpen visible={assayVisible}><AssaySheet visible={assayVisible} onClose={() => setAssayVisible(false)} send={sendWithFeedback} /></OnFirstOpen>

        <OnFirstOpen visible={selfMcpVisible}><SelfMcpSheet visible={selfMcpVisible} onClose={() => setSelfMcpVisible(false)} send={sendWithFeedback} /></OnFirstOpen>


        <OnFirstOpen visible={checkpointVisible}><CheckpointSheet visible={checkpointVisible} onClose={() => setCheckpointVisible(false)} send={sendWithFeedback} /></OnFirstOpen>


        <OnFirstOpen visible={pullRequestVisible}><PullRequestSheet visible={pullRequestVisible} onClose={() => setPullRequestVisible(false)} send={sendWithFeedback} /></OnFirstOpen>


        <OnFirstOpen visible={memoryVisible}><MemorySheet visible={memoryVisible} onClose={() => setMemoryVisible(false)} send={sendWithFeedback} /></OnFirstOpen>


        <OnFirstOpen visible={latticeVisible}><LatticeSheet visible={latticeVisible} onClose={() => setLatticeVisible(false)} send={sendWithFeedback} /></OnFirstOpen>
        <OnFirstOpen visible={lifecycleVisible}><SessionLifecycleSheet
          target={{
            id: sessionId,
            title,
            cwd: snapshot?.cwd ?? "",
            archived: false,
            running: !(closed || sessionEnded),
          }}
          visible={lifecycleVisible}
          onClose={() => setLifecycleVisible(false)}
        /></OnFirstOpen>


        {protocolWarning ? (
          <Banner tone="warn" message={protocolWarning} />
        ) : null}
        {publicExposure ? (
          <Banner
            tone="danger"
            message={`exposure: ${snapshot?.exposure} — anyone with this link can drive this session`}
          />
        ) : null}
        {closed || sessionEnded ? (
          <Banner tone="neutral" compact message="session ended — read-only. see History to review it" />
        ) : null}
        {/* A soft reconnect blip renders inline in the StatusStrip meta line (calmer than a
            full-width banner); only the hard unreachable state still earns a Banner. */}
        {hostRestarting && wsDown && !sessionEnded ? (
          <Banner tone="neutral" compact message="host restarting — this session is safe and will reconnect on its own" />
        ) : null}
        {unreachable && !hostRestarting ? (
          <Banner
            tone="danger"
            compact
            message="can't reach forge serve — check it's running. will keep retrying automatically."
          />
        ) : null}

          <View style={[gutter, styles.segmentedWrap]}>
            <TabStrip
              options={segmentOptions}
              value={activeSegment}
              onChange={onSegmentChange}
              testID="session-segmented"
              background={tokens.bg2}
            />
          </View>
          {projectNeedsInitialization ? (
            <Banner
              tone="warn"
              actionLabel={projectSetupRequested ? undefined : "Set it up for me"}
              message={projectSetupRequested ? "Setting up Forge for this project…" : "This project isn't set up for Forge — no project guidance or custom agents. Forge works best with an AGENTS.md and custom agents."}
              onAction={() => {
                if (sendWithFeedback({ kind: "prompt", text: "/init" })) setProjectSetupRequested(true);
              }}
              onDismiss={() => setProjectWarningDismissed(true)}
            />
          ) : null}
        </View>
      </SafeAreaView>

      <View style={gutter}>
        <GoalBanner snapshot={snapshot} send={sendWithFeedback} />
      </View>

      {KEEP_SEGMENTS_ALIVE ? (
        <SegmentsHostedContext.Provider value>
          <SegmentKeepAlive active={keptSegment} screens={SEGMENT_SCREENS} deferFirstFrame={active} />
          {terminalActive ? <SessionTerminal /> : null}
          <View style={keptSegment || terminalActive ? styles.none : styles.flex}>
            {hostRoutes ? (active && !onSegmentPath ? <PushedScreen pathname={pathname} /> : null) : <Slot />}
          </View>
        </SegmentsHostedContext.Provider>
      ) : (
        <Animated.View key={activeSegment} style={[styles.flex, segmentStyle]}>
          <Slot />
        </Animated.View>
      )}
      {/* HANDOFF(T4.1): overlay mirror — reads snapshot.overlay itself, no props. */}
      <OverlayHost />
    </View>
  );
}

export default function SessionLayout() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const keepMounted = useKeepSessionsMounted();

  // Wide desktop: `session/_layout` hosts the sessions (HostedSessions); this layout only has to
  // stay a navigator so the child routes resolve, and those render nothing.
  if (keepMounted) return <Slot />;

  // No id (shouldn't happen under `session/[id]`, but keeps this defensive): render a bare
  // back affordance rather than crashing on a null SessionProvider sessionId.
  if (!id) {
    return (
      <SafeAreaView style={styles.flex} edges={["top", "left", "right"]}>
        <IconButton
          icon={<ArrowLeft size={20} strokeWidth={1.75} />}
          onPress={() => goBackOr("/(tabs)")}
          accessibilityLabel="Back"
        />
      </SafeAreaView>
    );
  }

  return (
    <SessionProvider sessionId={id}>
      <SessionShell sessionId={id} active hostRoutes={false} />
    </SessionProvider>
  );
}

function PushedScreen({ pathname }: { pathname: string }) {
  if (pathname.endsWith("/workflows")) return <WorkflowLibraryScreen />;
  if (pathname.endsWith("/workflow")) return <WorkflowRunScreen />;
  return null;
}

/** Wide web/desktop host, rendered by `session/_layout`: the open session plus the most recent
 * others stay mounted (see KeptSessions); the `[id]` route tree sits beside them, hidden, because
 * its screens render nothing in this mode. */
export function HostedSessions() {
  const pathname = usePathname();
  const routedId = routedSessionId(pathname);
  // Off a session route (Settings pushed over it) the last open session stays the active one.
  const [routedActive, setRoutedActive] = useState(routedId);
  if (routedId != null && routedId !== routedActive) setRoutedActive(routedId);
  // A rail click names the session before the URL does (see activateSession).
  const override = useSessionActivationOverride();
  useEffect(() => settleActivation(routedId), [routedId, override]);
  const activeId = override ?? routedActive;
  return (
    <SegmentsHostedContext.Provider value>
      <RouteScreensHostedContext.Provider value>
        {activeId ? <KeptSessions activeId={activeId} /> : null}
        <View style={styles.none}>
          <Slot />
        </View>
      </RouteScreensHostedContext.Provider>
    </SegmentsHostedContext.Provider>
  );
}

// Wide web/desktop: the open session plus the most recent others (MAX_KEPT_SESSIONS in all) stay
// mounted. Hidden ones sit in <Activity mode="hidden">: display:none, state and DOM kept, effects
// cleaned up (so their socket is closed and nothing renders live snapshots) and re-renders
// deferred; showing one again re-runs its effects, which reconnect from the last revision.
// Memoised so a switch re-renders only the session leaving and the one arriving, not every kept one.
const KeptSession = React.memo(function KeptSession({ sessionId, active }: { sessionId: string; active: boolean }) {
  return (
    <SessionProvider sessionId={sessionId}>
      <SessionShell sessionId={sessionId} active={active} hostRoutes />
    </SessionProvider>
  );
});

function KeptSessions({ activeId }: { activeId: string }) {
  const [recent, setRecent] = useState<string[]>([activeId]);
  if (recent[0] !== activeId) {
    setRecent([activeId, ...recent.filter((sid) => sid !== activeId)].slice(0, MAX_KEPT_SESSIONS));
  }
  // A session the rail is hovering is mounted hidden before the click (it only takes a free
  // slot: it never evicts the open session or a more recent one).
  const warmRequest = useSyncExternalStore(subscribeSessionPrewarm, getSessionPrewarmRequest, getSessionPrewarmRequest);
  const [warm, setWarm] = useState<string[]>([]);
  const [seenRequest, setSeenRequest] = useState(warmRequest);
  if (warmRequest !== seenRequest) {
    setSeenRequest(warmRequest);
    if (warmRequest && !recent.includes(warmRequest.id)) {
      setWarm([warmRequest.id, ...warm.filter((sid) => sid !== warmRequest.id)].slice(0, MAX_KEPT_SESSIONS));
    }
  }
  const ids = [...recent, ...warm.filter((sid) => !recent.includes(sid))].slice(0, MAX_KEPT_SESSIONS);
  return (
    <>
      {ids.map((sid) => (
        <Activity key={sid} mode={sid === activeId ? "visible" : "hidden"}>
          <KeptSession sessionId={sid} active={sid === activeId} />
        </Activity>
      ))}
    </>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  none: { display: "none" },
  segmentedWrap: { paddingBottom: space.space8 },
});
