import { router } from "expo-router";
import React, { useMemo } from "react";
import { Pressable, RefreshControl, StyleSheet, Text } from "react-native";

import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { useSessionTree } from "../lib/queries";
import { GitBranch } from "lucide-react-native";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

export default function SessionTreeScreen() {
  const tokens = useTokens();
  const query = useSessionTree();
  const nodes = useMemo(() => query.data ?? [], [query.data]);
  const roots = nodes.filter((node) => node.forked_from == null);
  const children = (id: string) => nodes.filter((node) => node.forked_from === id);
  const renderNode = (id: string, depth: number): React.ReactNode => <React.Fragment key={id}>{[...children(id)].map((node) => <React.Fragment key={node.id}><Pressable onPress={() => router.push(`/session/${node.id}`)} accessibilityRole="button"><Card style={[styles.node, { marginLeft: depth * space.space12 }]}><Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{node.id}</Text><Text style={[type.sub, { color: tokens.ink3 }]}>{node.forked_at_seq == null ? "session root" : `forked at message ${node.forked_at_seq}`}</Text></Card></Pressable>{renderNode(node.id, depth + 1)}</React.Fragment>)}</React.Fragment>;
  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}><Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable><Text style={[type.title, { color: tokens.ink }]}>Session tree</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Forks and their conversation ancestry.</Text>{nodes.length === 0 && !query.isLoading ? <EmptyState icon={GitBranch} message="No session branches yet." /> : null}{roots.map((root) => <React.Fragment key={root.id}><Pressable onPress={() => router.push(`/session/${root.id}`)} accessibilityRole="button"><Card style={styles.node}><Text style={[type.body, { color: tokens.ink }]} numberOfLines={1}>{root.id}</Text><Text style={[type.sub, { color: tokens.ink3 }]}>session root</Text></Card></Pressable>{renderNode(root.id, 1)}</React.Fragment>)}</Screen>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, back: { fontSize: 15, fontWeight: "600" }, node: { gap: space.space4, marginBottom: space.space8 } });
