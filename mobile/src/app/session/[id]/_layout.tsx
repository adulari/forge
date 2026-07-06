// Session shell (BUILD_PLAN §6 "Session shell" / §7 Batch 2 W4). Owns the ONE
// `SessionProvider` (which owns the ONE `useSessionSocket` instance) for this session id —
// child segments (Chat/Tasks/Agents/Review/Overlay) render underneath via a nested `Stack`
// and never remount the provider, so the socket survives every segment switch. Header +
// status strip + Segmented sub-nav live here (UI_RULES.md #2 — one header pattern).
import * as Clipboard from "expo-clipboard";
import { Stack, useLocalSearchParams, useRouter, useSegments } from "expo-router";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Text, View } from "react-native";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";

import { Badge, Chip, EntranceView, Metric, Segmented, type SegmentedOption, StatusDot, type StatusDotState } from "../../../components/ui";
import { PermissionCard } from "../../../components/permissionCard";
import { QuestionCard } from "../../../components/questionCard";
import { colors } from "../../../lib/theme";
import { SessionProvider, useSessionCtx } from "../../../lib/sessionContext";

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
          <Stack.Screen name="overlay" options={{ presentation: "modal", animation: "slide_from_bottom" }} />
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
            if/else-if in renderActions, remote_assets/app.js). */}
        {snapshot?.permission_prompt || snapshot?.question ? (
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
        ) : null}
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
