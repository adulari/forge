// Session shell (BUILD_PLAN §6 "Session shell" / §7 Batch 2 W4). Owns the ONE
// `SessionProvider` (which owns the ONE `useSessionSocket` instance) for this session id —
// child segments (Chat/Tasks/Agents/Review/Overlay) render underneath via a nested `Stack`
// and never remount the provider, so the socket survives every segment switch. Header +
// status strip + Segmented sub-nav live here (UI_RULES.md #2 — one header pattern).
import * as Clipboard from "expo-clipboard";
import { Stack, useLocalSearchParams, useRouter, useSegments } from "expo-router";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Platform, Pressable, Text, View } from "react-native";
import Animated from "react-native-reanimated";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";

import { Badge, Chip, EntranceView, Metric, Segmented, type SegmentedOption, StatusDot, type StatusDotState } from "../../../components/ui";
import { PermissionCard } from "../../../components/permissionCard";
import { QuestionCard } from "../../../components/questionCard";
import { usePressScale } from "../../../lib/motion";
import { colors } from "../../../lib/theme";
import { SessionProvider, useSessionCtx } from "../../../lib/sessionContext";

function lightHaptic() {
  if (Platform.OS === "web") return;
  import("expo-haptics")
    .then((H) => H.impactAsync(H.ImpactFeedbackStyle.Light))
    .catch(() => {});
}

// Compact affordance shown instead of the generic QuestionCard when a plan-approval question
// is pending (BUILD_PLAN §6 Review "Build it"/"Cancel" flow) — the Review plan card is the
// single Approve/Cancel/Revise surface, so this just jumps there instead of duplicating it.
function PlanReadyJump({ onPress }: { onPress: () => void }) {
  const { style, onPressIn, onPressOut } = usePressScale();
  return (
    <Animated.View style={style}>
      <Pressable
        onPress={onPress}
        onPressIn={onPressIn}
        onPressOut={onPressOut}
        className="flex-row items-center gap-8 bg-panel border border-accent rounded-lg px-10 py-10"
        style={{ minHeight: 44 }}
      >
        <Text className="text-accent text-[15px] font-bold">⬡</Text>
        <View className="flex-1">
          <Text className="text-accent text-[14px] font-bold">Plan ready</Text>
          <Text className="text-dim text-[12px]">Tap to review &amp; approve</Text>
        </View>
        <Text className="text-dim text-[13px]">›</Text>
      </Pressable>
    </Animated.View>
  );
}

function baseName(p: string | undefined | null): string {
  if (!p) return "";
  const parts = p.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

function formatTokenCount(n: number): string {
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function contextToneClass(pct: number): { bar: string; text: string } {
  if (pct > 0.9) return { bar: "bg-no", text: "text-no" };
  if (pct > 0.7) return { bar: "bg-accent", text: "text-accent" };
  return { bar: "bg-dim", text: "text-dim" };
}

function ContextGauge({ tokens, limit }: { tokens: number; limit: number | null }) {
  if (!tokens) return null;
  if (!limit) {
    return (
      <Text className="text-dim text-[12px]" style={{ fontVariant: ["tabular-nums"] }}>
        {formatTokenCount(tokens)}
      </Text>
    );
  }
  const pct = Math.min(1, tokens / limit);
  const tone = contextToneClass(pct);
  return (
    <View className="items-end gap-2">
      <Text className={`text-[12px] ${tone.text}`} style={{ fontVariant: ["tabular-nums"] }}>
        {formatTokenCount(tokens)}/{formatTokenCount(limit)}
      </Text>
      <View className="w-[40px] h-[3px] rounded-full bg-borderSoft overflow-hidden">
        <View className={`h-[3px] rounded-full ${tone.bar}`} style={{ width: `${pct * 100}%` }} />
      </View>
    </View>
  );
}

type SegmentKey = "chat" | "tasks" | "agents" | "review";
const KNOWN_SEGMENTS: readonly string[] = ["tasks", "agents", "review"];

interface Toast {
  id: number;
  text: string;
}

function SessionShell() {
  const { sessionId, snapshot, connectionState, send } = useSessionCtx();
  const router = useRouter();
  const segments = useSegments();
  const insets = useSafeAreaInsets();

  const last = segments[segments.length - 1];
  const activeKey: SegmentKey = KNOWN_SEGMENTS.includes(last) ? (last as SegmentKey) : "chat";

  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);
  const pushToast = useCallback((text: string) => {
    const id = ++toastIdRef.current;
    setToasts((t) => [...t, { id, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
  }, []);

  // copy_text: put on the device clipboard the moment it changes to a non-null value, once.
  const lastCopyRef = useRef<string | null>(null);
  useEffect(() => {
    const ct = snapshot?.copy_text ?? null;
    if (ct && ct !== lastCopyRef.current) {
      lastCopyRef.current = ct;
      Clipboard.setStringAsync(ct).catch(() => {});
      pushToast("Copied to clipboard");
    }
    if (!ct) lastCopyRef.current = null;
  }, [snapshot?.copy_text, pushToast]);

  // notes: remote-facing notices render as transient toasts (only newly-seen ones).
  const lastNotesRef = useRef<string[]>([]);
  useEffect(() => {
    const notes = snapshot?.notes ?? [];
    const prevNotes = lastNotesRef.current;
    const fresh = notes.filter((n) => !prevNotes.includes(n));
    fresh.forEach((n) => pushToast(n));
    lastNotesRef.current = notes;
  }, [snapshot?.notes, pushToast]);

  // Overlay mirror: auto-present the modal the instant `snapshot.overlay` goes non-null, and
  // auto-dismiss it the instant the server clears it (a picker action completing, `/help`
  // toggled off, etc.) — this is the ONLY place that decides presentation; overlay.tsx itself
  // never navigates on its own except via explicit close.
  const overlayPresentedRef = useRef(false);
  const overlayCancelSentRef = useRef(false);
  const hasOverlay = !!snapshot?.overlay;
  useEffect(() => {
    if (hasOverlay && !overlayPresentedRef.current) {
      overlayPresentedRef.current = true;
      overlayCancelSentRef.current = false;
      router.push(`/session/${sessionId}/overlay`);
    } else if (!hasOverlay && overlayPresentedRef.current) {
      overlayPresentedRef.current = false;
      router.back();
    }
  }, [hasOverlay, sessionId, router]);

  // Native header close button (the overlay Stack.Screen below sets headerShown:true and
  // gestureEnabled:false, so this — plus Android hardware back inside overlay.tsx itself — is
  // the only way to dismiss it; both funnel through the same `overlay_cancel` intent).
  const handleOverlayClose = useCallback(() => {
    if (!overlayCancelSentRef.current) {
      overlayCancelSentRef.current = true;
      send({ kind: "overlay_cancel" });
    }
    router.back();
  }, [send, router]);

  const go = useCallback(
    (key: string) => {
      if (key === activeKey) return;
      const path = key === "chat" ? `/session/${sessionId}` : `/session/${sessionId}/${key}`;
      router.replace(path);
    },
    [activeKey, router, sessionId],
  );

  const title = snapshot?.title || `#${sessionId.slice(0, 8)}`;
  const cwdTail = baseName(snapshot?.cwd);
  const exposure = snapshot?.exposure ?? "";
  const isPublic = exposure.startsWith("public");
  const busy = snapshot?.busy ?? false;
  const waiting = !!(snapshot?.permission_prompt || snapshot?.question);
  const dotState: StatusDotState = !snapshot ? "idle-past" : busy ? "busy" : waiting ? "waiting" : "idle";
  const closed = snapshot?.closed ?? false;

  const segOptions: SegmentedOption[] = [
    { key: "chat", label: "Chat" },
    { key: "tasks", label: "Tasks", count: snapshot?.tasks.length ? snapshot.tasks.length : undefined },
    { key: "agents", label: "Agents", count: snapshot?.subagents.length ? snapshot.subagents.length : undefined },
    {
      key: "review",
      label: "Review",
      dot: !!(snapshot?.plan || (snapshot?.diff?.files?.length ?? 0) > 0),
    },
  ];

  return (
    <View className="flex-1 bg-bg">
      <SafeAreaView edges={["top"]} className="bg-panel border-b border-borderSoft">
        <View className="flex-row items-center gap-8 px-12 pt-8 pb-6">
          <StatusDot state={dotState} />
          <View className="flex-1">
            <Text numberOfLines={1} className="text-ink text-[16px] font-bold">
              {title}
            </Text>
            {cwdTail ? (
              <Text numberOfLines={1} ellipsizeMode="head" className="text-dim text-[12px] mt-2">
                {cwdTail}
              </Text>
            ) : null}
          </View>
          {isPublic ? <Badge label={exposure} tone="no" /> : null}
          {snapshot?.worktree ? <Badge label="⎇" tone="default" /> : null}
        </View>

        <View className="flex-row items-center gap-10 px-12 pb-8">
          <Badge label={`${snapshot?.tier ? `${snapshot.tier} · ` : ""}${snapshot?.model || "—"}`} tone="accent" />
          <Metric value={snapshot?.cost_usd ?? 0} format="cost" tone="ok" />
          <View className="flex-1" />
          <ContextGauge tokens={snapshot?.context_tokens ?? 0} limit={snapshot?.context_limit ?? null} />
          <Chip
            label="Stop"
            tone="danger"
            disabled={!busy}
            onPress={() => send({ kind: "interrupt" })}
          />
        </View>

        {connectionState === "reconnecting" || connectionState === "connecting" ? (
          <Text className="text-dim text-[12px] px-12 pb-6">
            {connectionState === "reconnecting" ? "reconnecting…" : "connecting…"}
          </Text>
        ) : null}
        {closed ? (
          <Text className="text-no text-[12px] px-12 pb-6">session ended</Text>
        ) : null}

        <View className="px-12 pb-8">
          <Segmented options={segOptions} value={activeKey} onChange={go} />
        </View>
      </SafeAreaView>

      <View className="flex-1">
        <Stack
          screenOptions={{
            headerShown: false,
            animation: "none",
            contentStyle: { backgroundColor: colors.bg },
          }}
        >
          <Stack.Screen name="index" />
          <Stack.Screen name="tasks" />
          <Stack.Screen name="agents" />
          <Stack.Screen name="review" />
          <Stack.Screen
            name="overlay"
            options={{
              presentation: "modal",
              animation: "slide_from_bottom",
              headerShown: true,
              headerBackVisible: false,
              gestureEnabled: false,
              title: snapshot?.overlay?.title || snapshot?.overlay?.kind || "Overlay",
              headerStyle: { backgroundColor: colors.panel },
              headerTitleStyle: { color: colors.ink, fontWeight: "700" },
              headerTintColor: colors.accent,
              headerRight: () => (
                <Pressable
                  onPress={handleOverlayClose}
                  hitSlop={8}
                  style={{ minWidth: 44, minHeight: 44, alignItems: "center", justifyContent: "center" }}
                >
                  <Text style={{ color: colors.dim, fontSize: 18 }}>✕</Text>
                </Pressable>
              ),
            }}
          />
        </Stack>

        {toasts.length ? (
          <View className="absolute top-8 left-12 right-12 z-10 gap-6" pointerEvents="none">
            {toasts.map((t) => (
              <View key={t.id} className="bg-bannerBg border border-border rounded-md px-10 py-8">
                <Text className="text-bannerInk text-[12px]" numberOfLines={2}>
                  {t.text}
                </Text>
              </View>
            ))}
          </View>
        ) : null}

        {/* Urgent action sheet: pinned above whatever segment (Chat/Tasks/Agents/Review) is
            showing, so a pending approval is never missed. Permission takes priority over a
            question if both are ever active at once (mirrors the web control page's
            if/else-if in renderActions, remote_assets/app.js). When a plan is in flight, the
            Review plan card (review.tsx PlanCard) is the single Approve/Cancel/Revise surface
            for it — this sheet must not ALSO render the raw question options (that was the
            duplicate-UI bug), so it collapses to a compact jump affordance instead, and that
            affordance itself is suppressed while already viewing Review (nothing to jump to —
            the full plan card is right there). Non-plan questions are unaffected. */}
        {(() => {
          const hasPlanQuestion = snapshot?.plan != null && snapshot?.question != null;
          const showPlanJump = hasPlanQuestion && activeKey !== "review";
          const showQuestionCard = snapshot?.question != null && !hasPlanQuestion;
          const showSheet = !!snapshot?.permission_prompt || showPlanJump || showQuestionCard;
          if (!showSheet || !snapshot) return null;
          return (
            <View
              className="absolute left-12 right-12 z-20"
              style={{ bottom: 12 + insets.bottom }}
              pointerEvents="box-none"
            >
              <EntranceView index={0}>
                {snapshot.permission_prompt ? (
                  <PermissionCard
                    permissionPrompt={snapshot.permission_prompt}
                    seq={snapshot.prompt_seq}
                    send={send}
                  />
                ) : showPlanJump ? (
                  <PlanReadyJump
                    onPress={() => {
                      lightHaptic();
                      go("review");
                    }}
                  />
                ) : (
                  <QuestionCard
                    question={snapshot.question as string}
                    options={snapshot.question_options}
                    allowOther={snapshot.question_allow_other}
                    seq={snapshot.prompt_seq}
                    send={send}
                  />
                )}
              </EntranceView>
            </View>
          );
        })()}
      </View>
    </View>
  );
}

export default function SessionLayout() {
  const { id } = useLocalSearchParams<{ id: string }>();
  if (!id) return null;
  return (
    <SessionProvider sessionId={id}>
      <SessionShell />
    </SessionProvider>
  );
}
