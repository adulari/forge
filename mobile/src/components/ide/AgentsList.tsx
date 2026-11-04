// The project's agents, needs-you first. Picking one scopes the change queue and the agent panel
// to it; "All agents" is the default and shows everything.
import { router } from "expo-router";
import { Layers, Plus } from "lucide-react-native";
import React, { useCallback, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { SessionRow } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { BoundedList } from "../ds/BoundedList";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { useIde } from "./IdeContext";
import { AgentDot, LineDelta, ROW_HEIGHT, SectionLabel, type AgentDotState } from "./ideUi";
import { rowBackground } from "./rowStyle";

const MAX_VISIBLE_ROWS = 7;

interface AgentItem {
  id: string;
  title: string;
  state: AgentDotState;
  adds: number;
  dels: number;
}

function stateOf(row: SessionRow): AgentDotState {
  if (row.waiting) return "waiting";
  if (row.busy) return "busy";
  return row.done_at != null ? "done" : "idle";
}

const AgentRow = React.memo(function AgentRow({
  id,
  title,
  state,
  adds,
  dels,
  selected,
  onPress,
}: AgentItem & { selected: boolean; onPress: (id: string) => void }) {
  const tokens = useTokens();
  const handle = useCallback(() => onPress(id), [id, onPress]);
  return (
    <Pressable
      onPress={handle}
      accessibilityRole="button"
      accessibilityState={{ selected }}
      accessibilityLabel={`${title}${state === "waiting" ? ", needs you" : ""}`}
      style={({ hovered, pressed }: { hovered?: boolean; pressed: boolean }) => [
        styles.row,
        { backgroundColor: rowBackground(tokens, { hovered, pressed, selected }) },
      ]}
    >
      <AgentDot state={state} />
      <Text numberOfLines={1} style={[styles.title, { color: state === "done" ? tokens.ink3 : tokens.ink }]}>
        {title}
      </Text>
      {state === "waiting" ? <Text style={[styles.needs, { color: tokens.accent }]}>Needs you</Text> : <LineDelta adds={adds} dels={dels} />}
    </Pressable>
  );
});

export function AgentsList() {
  const tokens = useTokens();
  const { sessions, scope, setScope, fleetLoading, project } = useIde();
  const items = useMemo<AgentItem[]>(
    () =>
      sessions.map((row) => ({
        id: row.id,
        title: row.title?.trim() || `session ${row.id.slice(0, 8)}`,
        state: stateOf(row),
        adds: row.diff_added ?? 0,
        dels: row.diff_removed ?? 0,
      })),
    [sessions],
  );
  const onPress = useCallback((id: string) => setScope(scope === id ? null : id), [scope, setScope]);
  const waiting = items.filter((item) => item.state === "waiting").length;
  const rowsShown = Math.min(items.length, MAX_VISIBLE_ROWS);

  return (
    <View>
      <SectionLabel right={waiting > 0 ? <Text style={[styles.count, { color: tokens.accent }]}>{waiting} waiting</Text> : <Text style={[styles.count, { color: tokens.ink4 }]}>{items.length}</Text>}>
        Agents
      </SectionLabel>
      {fleetLoading ? (
        <View style={styles.skeleton}>
          <Skeleton width="70%" height={10} />
          <Skeleton width="55%" height={10} />
        </View>
      ) : items.length === 0 ? (
        <View style={styles.empty}>
          <EmptyState
            icon={Layers}
            message={project ? `No agents on ${project} yet. Start one and it shows up here.` : "No agents yet. Start a session and it shows up here."}
            action={<Button label="New session" variant="secondary" icon={<Plus size={13} color={tokens.ink} />} onPress={() => router.push("/new-session")} />}
          />
        </View>
      ) : (
        <>
          <Pressable
            onPress={() => setScope(null)}
            accessibilityRole="button"
            accessibilityState={{ selected: scope == null }}
            style={({ hovered, pressed }: { hovered?: boolean; pressed: boolean }) => [
              styles.row,
              { backgroundColor: rowBackground(tokens, { hovered, pressed, selected: scope == null }) },
            ]}
          >
            <Layers size={13} strokeWidth={1.75} color={tokens.ink3} />
            <Text style={[styles.title, { color: scope == null ? tokens.ink : tokens.ink2 }]}>All agents</Text>
          </Pressable>
          <View style={{ height: rowsShown * ROW_HEIGHT }}>
            <BoundedList<AgentItem>
              data={items}
              keyExtractor={(item) => item.id}
              renderItem={({ item }) => <AgentRow {...item} selected={scope === item.id} onPress={onPress} />}
              getItemLayout={(_, index) => ({ length: ROW_HEIGHT, offset: ROW_HEIGHT * index, index })}
              ListEmptyComponent={<View />}
            />
          </View>
        </>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { height: ROW_HEIGHT, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
  title: { flex: 1, minWidth: 0, fontFamily: sansFamilyFor(500), fontSize: 12.5 },
  needs: { fontFamily: sansFamilyFor(600), fontSize: 10.5 },
  count: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  skeleton: { paddingHorizontal: space.space12, paddingVertical: space.space8, gap: space.space12 },
  empty: { paddingVertical: space.space16 },
});
