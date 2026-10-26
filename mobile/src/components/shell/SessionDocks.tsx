// Docks that show one slice of a session — Tasks, Agents, Plan. Each reuses the screen the
// session shell already renders for that segment, mounted inside its own SessionProvider for the
// focused pane's session (the same trick the secondary pane uses), so a dock follows focus
// instead of being tied to the routed session.
import { Bot, ClipboardList, ListChecks } from "lucide-react-native";
import React from "react";
import { ScrollView, StyleSheet, View } from "react-native";

import SessionAgents from "../../app/session/[id]/agents";
import SessionTasks from "../../app/session/[id]/tasks";
import { SessionProvider, useSessionCtx } from "../../lib/sessionContext";
import { space } from "../../theme/tokens";
import { EmptyState } from "../ds/EmptyState";
import { PlanCard } from "../review/PlanCard";

type SessionDockKind = "tasks" | "agents" | "plan";

const EMPTY: Record<SessionDockKind, { icon: typeof ListChecks; message: string }> = {
  tasks: { icon: ListChecks, message: "Open a session to see its task list here." },
  agents: { icon: Bot, message: "Open a session to see its subagents here." },
  plan: { icon: ClipboardList, message: "Open a session to see its plan here." },
};

function PlanBody() {
  const { snapshot, send } = useSessionCtx();
  const plan = snapshot?.plan ?? null;
  if (!plan) {
    return <EmptyState icon={ClipboardList} message="No plan yet. Ask Forge to /plan a task and it appears here." />;
  }
  return (
    <ScrollView contentContainerStyle={styles.plan}>
      <PlanCard
        plan={plan}
        question={snapshot?.question ?? null}
        questionOptions={snapshot?.question_options ?? []}
        promptSeq={snapshot?.prompt_seq ?? 0}
        send={send}
      />
    </ScrollView>
  );
}

export function SessionDock({ kind, sessionId }: { kind: SessionDockKind; sessionId: string | null }) {
  if (!sessionId) {
    const empty = EMPTY[kind];
    return <EmptyState icon={empty.icon} message={empty.message} />;
  }
  return (
    // Keyed so following focus to another session tears the socket down rather than retargeting it.
    <SessionProvider key={sessionId} sessionId={sessionId}>
      <View style={styles.fill}>
        {kind === "tasks" ? <SessionTasks /> : kind === "agents" ? <SessionAgents /> : <PlanBody />}
      </View>
    </SessionProvider>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  plan: { padding: space.space12 },
});
