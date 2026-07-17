// Hearth Session Tree — de-boxed branch timeline (core rule 1): forks + their ancestry
// as an indented left-rule tree, NOT a Card per node. `SessionTreeRow` is a plain REST
// ancestry query, so live status / worktree / model are joined from `useSessions()`
// (real, not fabricated) — the busy session lights its path with the ember HeatEdge and
// an Emberdot; everything else is a static ink dot. Two-column on medium+ (tree left,
// selected-node detail right) per the desktop prototype; single column stacks on compact.
import { router } from "expo-router";
import { ExternalLink, GitBranch } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { ActivityIndicator, Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { EmptyState } from "../components/ds/EmptyState";
import { HeatEdge } from "../components/ds/HeatEdge";
import { Screen } from "../components/ds/Screen";
import { StatusDot } from "../components/ds/StatusDot";
import { ForkSheet } from "../components/session/ForkSheet";
import { type SessionRow, type SessionTreeRow } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useSessions, useSessionTree } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space, type ColorTokens } from "../theme/tokens";
import { formatCost, formatRelativeTime, monoFamily, tabularNums, type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

interface TreeRow {
  node: SessionTreeRow;
  depth: number;
  orphaned: boolean;
}

type LiveState = "busy" | "waiting" | null;

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
const worktreeSeg = (path: string) => path.replace(/\/+$/, "").split("/").pop() ?? path;

/** The busy session and its ancestor chain — the "current path" the prototype heat-edges. */
function livePath(nodes: SessionTreeRow[], live: Map<string, LiveState>): Set<string> {
  const byId = new Map(nodes.map((node) => [node.id, node] as const));
  const path = new Set<string>();
  let cursor = nodes.find((node) => live.get(node.id) === "busy") ?? nodes.find((node) => live.get(node.id) === "waiting");
  const guard = new Set<string>();
  while (cursor && !guard.has(cursor.id)) {
    guard.add(cursor.id);
    path.add(cursor.id);
    cursor = cursor.forked_from ? byId.get(cursor.forked_from) : undefined;
  }
  return path;
}

export default function SessionTreeScreen() {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const query = useSessionTree();
  const sessions = useSessions();
  const { isCompact } = useBreakpoint();

  const nodes = useMemo(() => query.data ?? [], [query.data]);
  const rows = useMemo(() => flattenTree(nodes), [nodes]);
  const sessionById = useMemo(() => new Map((sessions.data ?? []).map((s) => [s.id, s] as const)), [sessions.data]);
  const liveById = useMemo(() => {
    const map = new Map<string, LiveState>();
    for (const s of sessions.data ?? []) map.set(s.id, s.busy ? "busy" : s.waiting ? "waiting" : null);
    return map;
  }, [sessions.data]);
  const path = useMemo(() => livePath(nodes, liveById), [nodes, liveById]);
  const currentNode = useMemo(
    () => nodes.find((n) => liveById.get(n.id) === "busy") ?? nodes.find((n) => liveById.get(n.id) === "waiting") ?? null,
    [nodes, liveById],
  );

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [forkTarget, setForkTarget] = useState<string | null>(null);
  const selected = selectedId ?? rows.find((r) => path.has(r.node.id))?.node.id ?? rows[0]?.node.id ?? null;
  const selectedRow = rows.find((r) => r.node.id === selected) ?? null;

  const openNode = (id: string) => router.push(`/session/${id}`);
  const showList = baseUrl && rows.length > 0;
  const currentForkId = currentNode?.id ?? rows.find((r) => path.has(r.node.id))?.node.id ?? rows[0]?.node.id ?? null;
  const headerName = currentNode ? titleFor(currentNode) : path.size > 0 ? "live path" : "ancestry";

  return (
    <DesktopDrillDown>
      <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
        <BackLink />
        <View style={[styles.columns, isCompact ? styles.columnsStacked : styles.columnsRow]}>
          <View style={styles.treeCol}>
            <View style={styles.heading}>
              <Text style={[typeScale.title, styles.headingTitle, { color: tokens.ink }]}>Session tree</Text>
              <Text style={[styles.mono, styles.headerName, { color: tokens.ink3 }]} numberOfLines={1}>{headerName}</Text>
            </View>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Forks branch from any turn — the original keeps running.</Text>

            {query.isLoading ? (
              <View style={styles.loading}><ActivityIndicator color={tokens.accent} /><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Loading session ancestry…</Text></View>
            ) : null}
            {query.isError ? (
              <Card><Text style={[typeScale.body, { color: tokens.danger }]}>Could not load the session tree.</Text><Button label="Retry" variant="secondary" onPress={() => void query.refetch()} /></Card>
            ) : null}
            {!baseUrl ? <EmptyState icon={GitBranch} message="Connect a server to view session branches." action={<Button label="Connect server" onPress={() => router.push("/connect")} />} /> : null}
            {baseUrl && rows.length === 0 && !query.isLoading && !query.isError ? <EmptyState icon={GitBranch} message="No session branches yet." /> : null}

            {showList ? (
              <View style={styles.list}>
                {rows.map(({ node, depth, orphaned }, index) => {
                  const isFork = node.forked_from != null && !orphaned;
                  const live = liveById.get(node.id) ?? null;
                  const onPath = path.has(node.id);
                  const isSelected = node.id === selected;
                  const session = sessionById.get(node.id) ?? null;
                  const relation = orphaned ? "original parent unavailable" : isFork ? `forked at message ${node.forked_at_seq ?? "—"}` : "session root";
                  const dimmed = path.size > 0 && !onPath && !isSelected;
                  return (
                    <View
                      key={node.id}
                      style={[
                        styles.rowWrap,
                        { marginLeft: Math.min(depth, 5) * space.space16 },
                        depth > 0 ? { borderLeftColor: tokens.border, borderLeftWidth: 1 } : null,
                        isSelected && !isCompact ? { backgroundColor: tokens.selection } : null,
                      ]}
                    >
                      {live ? <HeatEdge state={live} /> : depth === 0 && onPath ? <View style={[styles.rootRule, { backgroundColor: tokens.borderStrong }]} /> : null}
                      <Pressable
                        onPress={() => (isCompact ? openNode(node.id) : setSelectedId(node.id))}
                        accessibilityRole="button"
                        accessibilityLabel={`${isCompact ? "Open" : "Select"} ${titleFor(node)}`}
                        accessibilityState={{ selected: isSelected }}
                        style={[styles.row, { opacity: dimmed ? 0.6 : 1 }]}
                      >
                        <View style={styles.rowHeader}>
                          {live ? (
                            <StatusDot state={live} size={8} />
                          ) : isFork ? (
                            <GitBranch size={13} strokeWidth={1.75} color={onPath ? tokens.accent : tokens.ink4} />
                          ) : (
                            <View style={[styles.dot, { backgroundColor: onPath ? tokens.accent : tokens.ink3 }]} />
                          )}
                          <Text style={[typeScale.bodyBold, styles.rowTitle, { color: onPath || isSelected ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{titleFor(node)}</Text>
                          {live ? <Text style={[styles.mono, { color: tokens.accent }]}>{live === "busy" ? "current" : "waiting"}</Text> : <Text style={[styles.mono, { color: onPath ? tokens.accent : tokens.ink4 }]}>{shortId(node.id)}</Text>}
                        </View>
                        <Text style={[styles.mono, tabularNums, styles.rowMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                          {relation} · {formatRelativeTime(node.created_at * 1000)}{session?.worktree ? ` · wt ${worktreeSeg(session.worktree)}` : ""}
                        </Text>
                      </Pressable>
                      {isCompact ? (
                        <View style={styles.inlineActions}>
                          <Pressable onPress={() => openNode(node.id)} accessibilityRole="button" accessibilityLabel={`Open ${titleFor(node)}`} hitSlop={8}><Text style={[typeScale.meta, { color: tokens.accent }]}>Open</Text></Pressable>
                          <Pressable onPress={() => setForkTarget(node.id)} accessibilityRole="button" accessibilityLabel={`Fork from ${titleFor(node)}`} hitSlop={8}><Text style={[typeScale.meta, { color: tokens.ink2 }]}>Fork here</Text></Pressable>
                        </View>
                      ) : null}
                      {index < rows.length - 1 ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
                    </View>
                  );
                })}
              </View>
            ) : null}

            {showList && isCompact && currentForkId ? (
              <View style={styles.forkCta}>
                <Button
                  label="Fork from current turn"
                  onPress={() => setForkTarget(currentForkId)}
                  fullWidth
                  icon={<GitBranch size={15} strokeWidth={2} color={tokens.bg2} />}
                />
              </View>
            ) : null}
          </View>

          {showList && !isCompact && selectedRow ? (
            <View style={styles.detailCol}>
              <NodeDetail
                node={selectedRow.node}
                orphaned={selectedRow.orphaned}
                live={liveById.get(selectedRow.node.id) ?? null}
                session={sessionById.get(selectedRow.node.id) ?? null}
                tokens={tokens}
                onOpen={() => openNode(selectedRow.node.id)}
                onFork={() => setForkTarget(selectedRow.node.id)}
              />
            </View>
          ) : null}
        </View>
      </Screen>

      {forkTarget ? <ForkSheet visible onClose={() => setForkTarget(null)} sessionId={forkTarget} /> : null}
    </DesktopDrillDown>
  );
}

function NodeDetail({ node, orphaned, live, session, tokens, onOpen, onFork }: {
  node: SessionTreeRow;
  orphaned: boolean;
  live: LiveState;
  session: SessionRow | null;
  tokens: ColorTokens;
  onOpen: () => void;
  onFork: () => void;
}) {
  const isFork = node.forked_from != null && !orphaned;
  const relation = orphaned ? "original parent unavailable" : isFork ? `forked at message ${node.forked_at_seq ?? "—"}` : "session root";
  return (
    <View style={[styles.detailCard, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={styles.detailHead}>
        {live ? <StatusDot state={live} size={8} /> : isFork ? <GitBranch size={15} strokeWidth={1.75} color={tokens.ink4} /> : <View style={[styles.dot, { backgroundColor: tokens.ink3 }]} />}
        <Text style={[typeScale.headingBold, styles.rowTitle, { color: tokens.ink }]} numberOfLines={2}>{titleFor(node)}</Text>
      </View>
      <Text style={[styles.mono, { color: tokens.accent }]}>{shortId(node.id)}</Text>

      <View style={styles.detailMeta}>
        <DetailRow label="branch" value={relation} tokens={tokens} />
        <DetailRow label="created" value={formatRelativeTime(node.created_at * 1000)} tokens={tokens} />
        {live ? <DetailRow label="state" value={live === "busy" ? "running" : "waiting for you"} tokens={tokens} valueColor={tokens.accent} /> : null}
        {session?.model ? <DetailRow label="model" value={session.model} tokens={tokens} /> : null}
        {session ? <DetailRow label="cost" value={formatCost(session.cost_usd)} tokens={tokens} valueColor={tokens.success} /> : null}
        {session?.worktree ? <DetailRow label="worktree" value={worktreeSeg(session.worktree)} tokens={tokens} last /> : null}
      </View>

      <View style={styles.detailActions}>
        <Button label="Open session" onPress={onOpen} fullWidth icon={<ExternalLink size={16} strokeWidth={2} color={tokens.bg2} />} />
        <Button label="Fork from here" variant="secondary" onPress={onFork} fullWidth icon={<GitBranch size={16} strokeWidth={2} color={tokens.accent} />} />
      </View>
    </View>
  );
}

function DetailRow({ label, value, tokens, valueColor, last = false }: { label: string; value: string; tokens: ColorTokens; valueColor?: string; last?: boolean }) {
  return (
    <View style={[styles.metaRow, !last ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{label}</Text>
      <Text style={[styles.mono, tabularNums, styles.metaValue, { color: valueColor ?? tokens.ink2 }]} numberOfLines={1}>{value}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12, width: "100%", maxWidth: 1000, alignSelf: "center" },
  columns: { gap: space.space24 },
  columnsStacked: { flexDirection: "column" },
  columnsRow: { flexDirection: "row", alignItems: "flex-start" },
  treeCol: { flex: 1.1, minWidth: 0, gap: space.space4 },
  detailCol: { flex: 0.9, minWidth: 0 },
  heading: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  headingTitle: { flexShrink: 0 },
  headerName: { flexShrink: 1, textAlign: "right" },
  forkCta: { marginTop: space.space16 },
  loading: { alignItems: "center", paddingVertical: space.space32, gap: space.space12 },
  list: { marginTop: space.space8 },
  rowWrap: { position: "relative", borderRadius: radii.radius8, overflow: "hidden" },
  rootRule: { position: "absolute", left: 0, top: space.space8, bottom: space.space8, width: 2, borderRadius: 1 },
  row: { paddingLeft: space.space16, paddingRight: space.space8, paddingVertical: space.space12 },
  rowHeader: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  rowTitle: { flex: 1, minWidth: 0 },
  rowMeta: { marginTop: 3, marginLeft: 16 },
  inlineActions: { flexDirection: "row", gap: space.space16, paddingLeft: 32, paddingBottom: space.space12 },
  dot: { width: 7, height: 7, borderRadius: 3.5 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
  detailCard: { borderWidth: 1, borderRadius: radii.radius16, padding: space.space20, gap: space.space8 },
  detailHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  detailMeta: { marginTop: space.space8 },
  metaRow: { minHeight: 40, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space12 },
  metaValue: { flexShrink: 1, textAlign: "right" },
  detailActions: { marginTop: space.space12, gap: space.space8 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
});
