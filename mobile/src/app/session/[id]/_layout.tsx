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
import React, { useEffect, useMemo, useRef } from "react";
import { StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Banner } from "../../../components/ds/Banner";
import { IconButton } from "../../../components/ds/IconButton";
import { Segmented, type SegmentedOption } from "../../../components/ds/Segmented";
import { useToast } from "../../../components/ds/ToastHost";
import { OverlayHost } from "../../../components/overlay/OverlayHost";
import { SessionHeader } from "../../../components/session/SessionHeader";
import { StatusStrip } from "../../../components/session/StatusStrip";
import { useTurnCompleted } from "../../../lib/queries";
import { SessionProvider, useSessionCtx } from "../../../lib/sessionContext";
import { PROTOCOL_VERSION } from "../../../lib/ws";
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
  const { snapshot, connectionState } = useSessionCtx();

  // ARCHITECTURE §4.1.4: on the `busy` true->false edge, invalidate this session's history
  // query so the finalized turn appears from the store. The shell only needs to call the
  // hook to fire that side effect — segments consume useHistory independently.
  useTurnCompleted(snapshot);

  const lastCopyText = useRef<string | null>(null);
  const seenNoteCount = useRef(0);

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

  // notes: render each newly-appended note as a toast (Signal). Snapshot.notes only grows
  // within a connection, so a length-based watermark is enough to avoid replaying old ones.
  useEffect(() => {
    const notes = snapshot?.notes ?? [];
    for (let i = seenNoteCount.current; i < notes.length; i++) {
      toast.show(notes[i]);
    }
    seenNoteCount.current = notes.length;
  }, [snapshot?.notes, toast]);

  // Reset watermarks when the session id changes (new socket, fresh snapshot history).
  useEffect(() => {
    lastCopyText.current = null;
    seenNoteCount.current = 0;
  }, [sessionId]);

  const activeSegment = segmentFromPathname(pathname);

  const segmentOptions = useMemo<SegmentedOption<SegmentValue>[]>(() => {
    const taskCount = snapshot?.tasks.length ?? 0;
    const agentCount = snapshot?.subagents.length ?? 0;
    const reviewPending = snapshot?.plan != null || snapshot?.diff != null;
    return [
      { value: "chat", label: "Chat" },
      { value: "tasks", label: taskCount > 0 ? `Tasks ${taskCount}` : "Tasks" },
      { value: "agents", label: agentCount > 0 ? `Agents ${agentCount}` : "Agents" },
      { value: "review", label: reviewPending ? "Review •" : "Review" },
    ];
  }, [snapshot?.tasks.length, snapshot?.subagents.length, snapshot?.plan, snapshot?.diff]);

  const onSegmentChange = (value: SegmentValue) => {
    if (value === activeSegment) return;
    const suffix = SEGMENT_SUFFIX[value];
    router.replace(`/session/${sessionId}${suffix ? `/${suffix}` : ""}` as never);
  };

  const closed = snapshot?.closed ?? false;
  const protocolMismatch = snapshot != null && snapshot.protocol !== PROTOCOL_VERSION;
  const publicExposure = (snapshot?.exposure ?? "").startsWith("public");
  const reconnecting = connectionState === "reconnecting";

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
  // DESIGN_SYSTEM.md §7 expanded: "Chat content column max-width 840". The full spec calls for
  // session/[id] to render inline as the right pane of a persistent Fleet rail (master-detail),
  // but that's a routing-architecture change (session/[id] is a sibling Stack route outside
  // (tabs) on purpose — see the HANDOFF in (tabs)/_layout.tsx) out of this fix's bounded scope.
  // Until that lands, center a capped column at the expanded breakpoint so the shell doesn't
  // stretch header/segmented/chat content edge-to-edge across a wide desktop viewport.
  const wideCol = isExpanded ? styles.wideCol : null;

  return (
    <View style={[styles.flex, { backgroundColor: tokens.bg1 }]}>
      <SafeAreaView edges={["top", "left", "right"]} style={{ backgroundColor: tokens.bg1 }}>
        <View style={[gutter, wideCol]}>
          <SessionHeader
            title={snapshot?.title || `session ${sessionId.slice(0, 8)}`}
            cwd={snapshot?.cwd ?? sessionId}
            worktree={snapshot?.worktree ?? null}
            exposure={snapshot?.exposure ?? "loopback"}
            onBack={() => router.back()}
          />
        </View>

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
        {reconnecting ? <Banner tone="neutral" compact message="reconnecting…" /> : null}

        <View style={[gutter, wideCol]}>
          <StatusStrip
            state={statusState}
            tier={snapshot?.tier ?? null}
            model={snapshot?.model ?? "—"}
            temper={snapshot?.temper ?? "—"}
            costUsd={snapshot?.cost_usd ?? 0}
            contextTokens={snapshot?.context_tokens ?? 0}
            contextLimit={snapshot?.context_limit ?? null}
          />
        </View>

        <View style={[gutter, styles.segmentedWrap, wideCol]}>
          <Segmented
            options={segmentOptions}
            value={activeSegment}
            onChange={onSegmentChange}
            testID="session-segmented"
          />
        </View>
      </SafeAreaView>

      <View style={[styles.flex, wideCol]}>
        <Slot />
      </View>
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
          onPress={() => router.back()}
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
  wideCol: { width: "100%", maxWidth: 840, alignSelf: "center" },
});
