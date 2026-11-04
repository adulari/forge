// Right panel: the agent in view. Header (model, state, cost), the needs-you card the chat uses,
// a compact timeline, tasks, and the real composer. It mounts one session socket, for this agent
// only; switching agents tears it down.
import { Check, Circle, CircleDot, Users } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { SessionProvider, useSessionCtx } from "../../lib/sessionContext";
import { newClientMsgId } from "../../lib/offlineQueue";
import { useHistory, useLiveHistoryRefresh } from "../../lib/queries";
import { formatCost , monoFamily, sansFamilyFor } from "../../theme/typography";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import CardSlot from "../chat/CardSlot";
import { Composer } from "../chat/Composer";
import type { SentAttachment } from "../chat/attach";
import { EmptyState } from "../ds/EmptyState";
import { ProviderGlyph } from "../ds/ProviderGlyph";
import { useToast } from "../ds/ToastHost";
import { shortModelLabel } from "../fleet/shortModelLabel";
import { useIde } from "./IdeContext";
import { Timeline } from "./Timeline";
import { AgentDot, SectionLabel } from "./ideUi";

export function AgentPanel() {
  const tokens = useTokens();
  const { panelSessionId } = useIde();
  return (
    <View style={[styles.root, { backgroundColor: tokens.bg1 }]}>
      {panelSessionId ? (
        <SessionProvider key={panelSessionId} sessionId={panelSessionId}>
          <Body sessionId={panelSessionId} />
        </SessionProvider>
      ) : (
        <EmptyState icon={Users} message="Pick an agent to see what it is doing and message it." />
      )}
    </View>
  );
}

function Body({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const { snapshot, send, connectionState, focusComposer } = useSessionCtx();
  const { panelSession, composerSignal } = useIde();

  useEffect(() => {
    if (composerSignal > 0) focusComposer();
    // Only a new request moves focus; switching agents must not steal it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [composerSignal]);

  // The history page the timeline reads is refreshed while the agent works.
  useHistory(sessionId, { includeTools: true });
  const liveSignal = useMemo(
    () => (snapshot?.transcript_rows ?? []).slice(-2).map((row) => `${row.kind}:${row.text.length}`).join("|"),
    [snapshot?.transcript_rows],
  );
  useLiveHistoryRefresh(sessionId, snapshot?.busy ?? false, liveSignal, true);

  const online = connectionState === "open";
  const handleSend = useCallback(
    (text: string, attachments: SentAttachment[]): boolean => {
      const withPath = attachments.filter((a): a is SentAttachment & { path: string } => a.path != null).map((a) => ({ path: a.path, image: a.image }));
      const sent = send({ kind: "prompt", text, attachments: withPath, client_msg_id: newClientMsgId() });
      if (!sent) toast.show("Not connected to this agent. Try again in a moment.", { tone: "danger" });
      return sent;
    },
    [send, toast],
  );
  const handleInterrupt = useCallback(() => {
    send({ kind: "interrupt" });
  }, [send]);

  const waiting = snapshot?.permission_prompt != null || snapshot?.question != null || !!panelSession?.waiting;
  const busy = snapshot?.busy ?? panelSession?.busy ?? false;
  const model = snapshot?.model ?? panelSession?.model ?? "";
  const cost = snapshot?.cost_usd ?? panelSession?.cost_usd ?? 0;
  const tasks = snapshot?.tasks ?? [];
  const done = tasks.filter((task) => task.status === "done").length;
  const readOnly = !!panelSession?.read_only;

  return (
    <View style={styles.fill}>
      <View style={[styles.header, { borderBottomColor: tokens.hairline }]}>
        <View style={styles.titleRow}>
          <AgentDot state={waiting ? "waiting" : busy ? "busy" : "idle"} />
          <Text numberOfLines={1} style={[styles.title, { color: tokens.ink }]}>
            {panelSession?.title?.trim() || `session ${sessionId.slice(0, 8)}`}
          </Text>
        </View>
        <View style={styles.metaRow}>
          <ProviderGlyph provider={model} size={11} color={tokens.ink3} />
          <Text numberOfLines={1} style={[styles.meta, { color: tokens.ink3, flexShrink: 1 }]}>
            {shortModelLabel(model)}
          </Text>
          <Text style={[styles.meta, { color: waiting ? tokens.accent : tokens.ink3 }]}>
            {"· "}
            {waiting ? "Needs you" : busy ? (snapshot?.activity || "Working") : "Idle"}
          </Text>
          <View style={styles.spacer} />
          <Text style={[styles.meta, { color: tokens.ink3 }]}>{formatCost(cost)}</Text>
        </View>
      </View>

      <CardSlot />

      <SectionLabel>Activity</SectionLabel>
      <View style={styles.timeline}>
        <Timeline sessionId={sessionId} />
      </View>

      {tasks.length > 0 ? (
        <View style={[styles.tasks, { borderTopColor: tokens.hairline }]}>
          <SectionLabel right={<Text style={[styles.meta, { color: tokens.ink4 }]}>{`${done}/${tasks.length}`}</Text>}>Tasks</SectionLabel>
          {tasks.slice(0, 6).map((task, index) => (
            <View key={`${index}:${task.title}`} style={styles.task}>
              {task.status === "done" ? (
                <Check size={12} strokeWidth={2} color={tokens.ink3} />
              ) : task.status === "in_progress" ? (
                <CircleDot size={12} strokeWidth={1.75} color={tokens.ink} />
              ) : (
                <Circle size={12} strokeWidth={1.75} color={tokens.ink4} />
              )}
              <Text numberOfLines={1} style={[styles.taskText, { color: task.status === "done" ? tokens.ink3 : tokens.ink2 }]}>
                {task.title}
              </Text>
            </View>
          ))}
          {tasks.length > 6 ? <Text style={[styles.meta, styles.more, { color: tokens.ink4 }]}>{`+${tasks.length - 6} more`}</Text> : null}
        </View>
      ) : null}

      {readOnly ? (
        <View style={[styles.readOnly, { borderTopColor: tokens.hairline }]}>
          <Text style={[styles.meta, { color: tokens.ink3 }]}>View only. This session runs in a terminal.</Text>
        </View>
      ) : (
        <Composer
          sessionId={sessionId}
          busy={busy}
          online={online}
          suggestedPrompt={snapshot?.suggested_prompt ?? null}
          model={snapshot?.model ?? null}
          modelPinned={snapshot?.model_pinned}
          effort={snapshot?.effort ?? null}
          temper={snapshot?.temper ?? null}
          onSend={handleSend}
          onInterrupt={handleInterrupt}
        />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minWidth: 0, minHeight: 0 },
  fill: { flex: 1, minHeight: 0 },
  header: { paddingHorizontal: space.space16, paddingTop: space.space12, paddingBottom: space.space12, gap: 4, borderBottomWidth: StyleSheet.hairlineWidth },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(600), fontSize: 13.5, letterSpacing: -0.1 },
  metaRow: { flexDirection: "row", alignItems: "center", gap: 5, paddingLeft: 15 },
  meta: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  spacer: { flex: 1 },
  timeline: { flex: 1, minHeight: 80 },
  tasks: { borderTopWidth: StyleSheet.hairlineWidth, paddingBottom: space.space8 },
  task: { height: 24, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space16 },
  taskText: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(400), fontSize: 12 },
  more: { paddingHorizontal: space.space16, paddingTop: 2 },
  readOnly: { padding: space.space16, borderTopWidth: StyleSheet.hairlineWidth },
});
