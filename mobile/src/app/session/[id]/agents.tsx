// Agents segment (BUILD_PLAN §6 "Agents" / §7 Batch 3 W7). Renders `snapshot.subagents` live
// from the shared session socket. Mirrors remote_assets/app.js's `renderAgents` card shape
// (agent name · model · done cost, task line, dim mono `last` line, opacity .7 when done —
// see remote_assets/styles.css `.agent`), expressed via the ui.tsx primitives per BUILD_PLAN.
// `subagents` carries no nesting/depth field on the wire — cards render as a flat list.
import React, { useCallback, useMemo } from "react";
import { Platform, Text, View } from "react-native";

import {
  Badge,
  BoundedList,
  Card,
  EmptyState,
  Loading,
  Metric,
  StatusDot,
} from "../../../components/ui";
import { useSessionCtx } from "../../../lib/sessionContext";
import type { SnapshotSubagent } from "../../../lib/ws";

const MONO_FONT = Platform.select({
  ios: "Menlo",
  android: "monospace",
  default: "ui-monospace",
});

interface AgentRow extends SnapshotSubagent {
  id: string;
}

function AgentCardBase({ item }: { item: AgentRow }) {
  return (
    <Card variant="feature" className={item.done ? "mb-8 gap-6 opacity-70" : "mb-8 gap-6"}>
      <View className="flex-row items-center gap-8">
        <StatusDot state={item.done ? "idle" : "busy"} />
        <Text numberOfLines={1} className="flex-1 text-accent text-[13px] font-semibold">
          {item.agent || "agent"}
        </Text>
        {item.model ? <Badge label={item.model} /> : null}
        <Metric value={item.cost} format="cost" tone="ok" />
      </View>
      {item.task ? (
        <Text numberOfLines={2} className="text-ink text-[13px]">
          {item.task}
        </Text>
      ) : null}
      {item.last ? (
        <View className="bg-codeBg rounded-md px-8 py-6">
          <Text
            numberOfLines={2}
            className="text-dim text-[12px]"
            style={{ fontFamily: MONO_FONT, lineHeight: 17 }}
          >
            {item.last}
          </Text>
        </View>
      ) : null}
    </Card>
  );
}

const AgentCard = React.memo(AgentCardBase, (prev, next) => {
  const a = prev.item;
  const b = next.item;
  return (
    a.id === b.id &&
    a.agent === b.agent &&
    a.task === b.task &&
    a.model === b.model &&
    a.last === b.last &&
    a.done === b.done &&
    a.cost === b.cost
  );
});

export default function AgentsScreen() {
  const { snapshot } = useSessionCtx();

  const data = useMemo<AgentRow[]>(
    () => (snapshot?.subagents ?? []).map((a, i) => ({ ...a, id: `${i}:${a.agent}` })),
    [snapshot?.subagents],
  );

  const keyExtractor = useCallback((item: AgentRow) => item.id, []);
  const renderItem = useCallback(
    ({ item }: { item: AgentRow }) => <AgentCard item={item} />,
    [],
  );
  const emptyComponent = useMemo(
    () => <EmptyState title="No subagents running." />,
    [],
  );

  if (!snapshot) {
    return (
      <View className="flex-1">
        <Loading label="Connecting to session…" />
      </View>
    );
  }

  return (
    <View className="flex-1">
      <BoundedList
        data={data}
        keyExtractor={keyExtractor}
        renderItem={renderItem}
        ListEmptyComponent={emptyComponent}
      />
    </View>
  );
}
