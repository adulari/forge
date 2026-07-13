import { router } from "expo-router";
import { GitBranch } from "lucide-react-native";
import React, { useMemo } from "react";
import { ActivityIndicator, Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { type SessionTreeRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useSessionTree } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { formatRelativeTime, type } from "../theme/typography";

interface TreeRow {
  node: SessionTreeRow;
  depth: number;
  orphaned: boolean;
}

function flattenTree(nodes: SessionTreeRow[]): TreeRow[] {
  const ids = new Set(nodes.map((node) => node.id));
  const children = new Map<string, SessionTreeRow[]>();
  const roots: SessionTreeRow[] = [];
  for (const node of nodes) {
    if (node.forked_from == null || !ids.has(node.forked_from)) {
      roots.push(node);
      continue;
    }
    const siblings = children.get(node.forked_from) ?? [];
    siblings.push(node);
    children.set(node.forked_from, siblings);
  }

  const rows: TreeRow[] = [];
  const visited = new Set<string>();
  const visit = (node: SessionTreeRow, depth: number, ancestors: ReadonlySet<string>, orphaned = false) => {
    if (ancestors.has(node.id) || visited.has(node.id)) return;
    visited.add(node.id);
    rows.push({ node, depth, orphaned });
    const nextAncestors = new Set(ancestors).add(node.id);
    for (const child of children.get(node.id) ?? []) visit(child, depth + 1, nextAncestors);
  };
  for (const root of roots) visit(root, 0, new Set(), root.forked_from != null);
  for (const node of nodes) if (!visited.has(node.id)) visit(node, 0, new Set(), true);
  return rows;
}

const shortId = (id: string) => id.slice(0, 8);
const titleFor = (node: SessionTreeRow) => node.title?.trim() || `Untitled session · ${shortId(node.id)}`;

export default function SessionTreeScreen() {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const query = useSessionTree();
  const rows = useMemo(() => flattenTree(query.data ?? []), [query.data]);

  return (
    <DesktopDrillDown>
      <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
        <BackLink />
        <Text style={[type.title, { color: tokens.ink }]}>Session tree</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>Forks and their conversation ancestry.</Text>
        {query.isLoading ? <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[type.sub, { color: tokens.ink3 }]}>Loading session ancestry…</Text></View> : null}
        {query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load the session tree.</Text><Button label="Retry" variant="secondary" onPress={() => void query.refetch()} /></Card> : null}
        {!baseUrl ? <EmptyState icon={GitBranch} message="Connect a server to view session branches." action={<Button label="Connect server" onPress={() => router.push("/connect")} />} /> : null}
        {baseUrl && rows.length === 0 && !query.isLoading && !query.isError ? <EmptyState icon={GitBranch} message="No session branches yet." /> : null}
        {baseUrl ? rows.map(({ node, depth, orphaned }) => {
          const isFork = node.forked_from != null && !orphaned;
          const relation = orphaned ? "original parent unavailable" : isFork ? `forked at message ${node.forked_at_seq ?? "—"}` : "session root";
          return (
            <Pressable key={node.id} onPress={() => router.push(`/session/${node.id}`)} accessibilityRole="button" accessibilityLabel={`Open ${titleFor(node)}`}>
              <View style={[styles.branch, { marginLeft: Math.min(depth, 6) * space.space16, borderLeftColor: depth > 0 ? tokens.border : "transparent" }]}>
                <Card style={styles.node}>
                  <Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{titleFor(node)}</Text>
                  <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={1}>{relation} · {formatRelativeTime(node.created_at * 1000)} · {shortId(node.id)}</Text>
                </Card>
              </View>
            </Pressable>
          );
        }) : null}
      </Screen>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  loading: { alignItems: "center", paddingVertical: space.space32, gap: space.space12 },
  branch: { borderLeftWidth: 1, paddingLeft: space.space8 },
  node: { gap: space.space4, marginBottom: space.space8 },
});
