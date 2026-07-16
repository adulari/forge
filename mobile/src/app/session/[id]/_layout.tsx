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
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withTiming } from "react-native-reanimated";
import { StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Banner } from "../../../components/ds/Banner";
import { IconButton } from "../../../components/ds/IconButton";
import { Segmented, type SegmentedOption } from "../../../components/ds/Segmented";
import { useToast } from "../../../components/ds/ToastHost";
import { OverlayHost } from "../../../components/overlay/OverlayHost";
import { usePalette } from "../../../components/overlay/CommandPalette";
import { SessionHeader } from "../../../components/session/SessionHeader";
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
import { goBackOr } from "../../../lib/nav";
import { useHotkey } from "../../../lib/shortcuts";
import { useHistory, useSessionWeeklyDelta, useTurnCompleted } from "../../../lib/queries";
import { SessionProvider, useSessionCtx } from "../../../lib/sessionContext";
import { PROTOCOL_VERSION } from "../../../lib/ws";
import { durations, easings } from "../../../theme/motion";
import { useTokens } from "../../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../../theme/tokens";
import { useBreakpoint } from "../../../theme/useBreakpoint";

type SegmentValue = "chat" | "tasks" | "agents" | "review";

// Path suffix appended to `/session/{id}` for each segment ("" = the index/Chat route).
const SEGMENT_SUFFIX: Record<SegmentValue, string> = {
  chat: "",
  tasks: "tasks",
  agents: "agents",
  review: "review",
};

function segmentFromPathname(pathname: string): SegmentValue {
  const last = pathname.split("/").filter(Boolean).pop();
  if (last === "tasks" || last === "agents" || last === "review") return last;
  return "chat";
}

function SessionShell({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const pathname = usePathname();
  const { isCompact, isExpanded } = useBreakpoint();
  const { snapshot, connectionState, send, setHeaderHeight, baseUrl, focusComposer } = useSessionCtx();
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
  const { data: sessionHistory } = useHistory(sessionId);
  const weekly = useSessionWeeklyDelta(sessionId);
  const latestAssistantModel = useMemo(
    () => (sessionHistory?.pages ?? []).flat().find((row) => row.role === "assistant" && row.model)?.model ?? null,
    [sessionHistory],
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

  const activeSegment = segmentFromPathname(pathname);

  const segmentOptions = useMemo<SegmentedOption<SegmentValue>[]>(() => {
    const taskCount = snapshot?.tasks.length ?? 0;
    const agentCount = snapshot?.subagents.length ?? 0;
    const reviewPending = snapshot?.plan != null || snapshot?.diff != null;
    return [
      { value: "chat", label: "Chat" },
      { value: "tasks", label: "Tasks", badge: taskCount || undefined },
      { value: "agents", label: "Agents", badge: agentCount || undefined },
      { value: "review", label: "Review", dot: reviewPending },
    ];
  }, [snapshot?.tasks.length, snapshot?.subagents.length, snapshot?.plan, snapshot?.diff]);

  const reduced = useReducedMotion();
  const segmentOpacity = useSharedValue(1);
  const segmentY = useSharedValue(0);
  useEffect(() => {
    if (reduced) { segmentOpacity.value = 1; segmentY.value = 0; return; }
    segmentOpacity.value = 0;
    segmentY.value = 6;
    segmentOpacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
    segmentY.value = withTiming(0, { duration: durations.base, easing: easings.standard });
  }, [activeSegment, reduced, segmentOpacity, segmentY]);
  const segmentStyle = useAnimatedStyle(() => ({ opacity: segmentOpacity.value, transform: [{ translateY: segmentY.value }] }));

  const onSegmentChange = useCallback(
    (value: SegmentValue) => {
      if (value === activeSegment) return;
      const suffix = SEGMENT_SUFFIX[value];
      router.replace(`/session/${sessionId}${suffix ? `/${suffix}` : ""}` as never);
    },
    [activeSegment, sessionId],
  );

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
  useHotkey("c", () => onSegmentChange("chat"), { alt: true });
  useHotkey("t", () => onSegmentChange("tasks"), { alt: true });
  useHotkey("a", () => onSegmentChange("agents"), { alt: true });
  useHotkey("r", () => onSegmentChange("review"), { alt: true });
  useHotkey("e", focusComposer, { meta: true });
  useHotkey(".", interrupt, { meta: true });

  const closed = snapshot?.closed ?? false;
  const protocolMismatch = snapshot != null && snapshot.protocol !== PROTOCOL_VERSION;
  const publicExposure = (snapshot?.exposure ?? "").startsWith("public");
  const reconnecting = connectionState === "reconnecting";
  const unreachable = connectionState === "unreachable";
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
          <SessionHeader
            title={snapshot?.title || `session ${sessionId.slice(0, 8)}`}
            cwd={snapshot?.cwd ?? sessionId}
            worktree={snapshot?.worktree ?? null}
            exposure={snapshot?.exposure ?? "loopback"}
            onBack={() => goBackOr("/(tabs)")}
            showBack={!isExpanded}
            onNewHere={() => router.push({ pathname: "/new-session", params: { cwd: snapshot?.cwd ?? "" } })}
            onPalette={openPalette}
            onDuel={() => setDuelVisible(true)}
            onReplay={() => router.push(`/session/${sessionId}/replay`)}
            onPlan={() => setPlanVisible(true)}

            onFork={() => setForkVisible(true)}

            onInit={() => setInitVisible(true)}

            onAssay={() => setAssayVisible(true)}

            onSelfMcp={() => setSelfMcpVisible(true)}


            onCheckpoint={() => setCheckpointVisible(true)}


            onPullRequest={() => setPullRequestVisible(true)}


            onMemory={() => setMemoryVisible(true)}


            onLattice={() => setLatticeVisible(true)}
          />
          </View>

        <DuelSheet visible={duelVisible} onClose={() => setDuelVisible(false)} send={sendWithFeedback} />
        <PlanSheet visible={planVisible} onClose={() => setPlanVisible(false)} send={sendWithFeedback} />

        <ForkSheet visible={forkVisible} onClose={() => setForkVisible(false)} sessionId={sessionId} />

        <InitProjectSheet visible={initVisible} onClose={() => setInitVisible(false)} send={sendWithFeedback} />

        <AssaySheet visible={assayVisible} onClose={() => setAssayVisible(false)} send={sendWithFeedback} />

        <SelfMcpSheet visible={selfMcpVisible} onClose={() => setSelfMcpVisible(false)} send={sendWithFeedback} />


        <CheckpointSheet visible={checkpointVisible} onClose={() => setCheckpointVisible(false)} send={sendWithFeedback} />


        <PullRequestSheet visible={pullRequestVisible} onClose={() => setPullRequestVisible(false)} send={sendWithFeedback} />


        <MemorySheet visible={memoryVisible} onClose={() => setMemoryVisible(false)} send={sendWithFeedback} />


        <LatticeSheet visible={latticeVisible} onClose={() => setLatticeVisible(false)} send={sendWithFeedback} />

        {protocolMismatch ? (
          <Banner tone="warn" message="protocol mismatch — update Forge or the app" />
        ) : null}
        {publicExposure ? (
          <Banner
            tone="danger"
            message={`exposure: ${snapshot?.exposure} — anyone with this link can drive this session`}
          />
        ) : null}
        {closed ? <Banner tone="danger" message="session ended — see History to review it" /> : null}
        {unreachable ? (
          <Banner
            tone="danger"
            compact
            message="can't reach forge serve — check it's running. will keep retrying automatically."
          />
        ) : reconnecting ? (
          <Banner tone="neutral" compact message="reconnecting…" />
        ) : null}

        <View style={gutter}>
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
            weekly={weekly.mode === "subscription" ? { provider: weekly.provider, deltaPct: weekly.deltaPct } : null}
          />
        </View>

          <View style={[gutter, styles.segmentedWrap]}>
            <Segmented
              options={segmentOptions}
              value={activeSegment}
              onChange={onSegmentChange}
              testID="session-segmented"
              flush
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

      <Animated.View key={activeSegment} style={[styles.flex, segmentStyle]}>
        <Slot />
      </Animated.View>
      {/* HANDOFF(T4.1): overlay mirror — reads snapshot.overlay itself, no props. */}
      <OverlayHost />
    </View>
  );
}

export default function SessionLayout() {
  const { id } = useLocalSearchParams<{ id: string }>();

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
      <SessionShell sessionId={id} />
    </SessionProvider>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  segmentedWrap: { paddingBottom: space.space8 },
});
