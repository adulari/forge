import { router } from "expo-router";
import { GitBranch } from "lucide-react-native";
import React, { useMemo } from "react";
import { Pressable, RefreshControl, StyleSheet, Text } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { type SessionTreeRow } from "../lib/api";
import { useSessionTree } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

interface TreeRow {
  node: SessionTreeRow;
  depth: number;
}

function flattenTree(nodes: SessionTreeRow[]): TreeRow[] {
  const children = new Map<string | null, SessionTreeRow[]>();
  for (const node of nodes) {
    const siblings = children.get(node.forked_from) ?? [];
    siblings.push(node);
    children.set(node.forked_from, siblings);
  }

  const rows: TreeRow[] = [];
  const visit = (parentId: string | null, depth: number, ancestors: ReadonlySet<string>) => {
    for (const node of children.get(parentId) ?? []) {
      if (ancestors.has(node.id)) continue;
      rows.push({ node, depth });
      visit(node.id, depth + 1, new Set(ancestors).add(node.id));
    }
  };
  visit(null, 0, new Set());
  return rows;
}

export default function SessionTreeScreen() {
  const tokens = useTokens();
  const query = useSessionTree();
  const rows = useMemo(() => flattenTree(query.data ?? []), [query.data]);

  return (
    <DesktopDrillDown>
      <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
      <Pressable onPress={() => router.back()} accessibilityRole="button">
        <Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text>
      </Pressable>
      <Text style={[type.title, { color: tokens.ink }]}>Session tree</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>Forks and their conversation ancestry.</Text>
      {query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load the session tree. Pull to retry.</Text></Card> : null}
      {rows.length === 0 && !query.isLoading ? <EmptyState icon={GitBranch} message="No session branches yet." /> : null}
      {rows.map(({ node, depth }) => (
        <Pressable key={node.id} onPress={() => router.push(`/session/${node.id}`)} accessibilityRole="button">
          <Card style={[styles.node, { marginLeft: depth * space.space12 }]}>
            <Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{node.id}</Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>{node.forked_at_seq == null ? "session root" : `forked at message ${node.forked_at_seq}`}</Text>
          </Card>
        </Pressable>
      ))}
      </Screen>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  node: { gap: space.space4, marginBottom: space.space8 },
});
