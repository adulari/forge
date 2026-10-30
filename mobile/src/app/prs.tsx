// Pull requests across every project the host runs sessions in. One `gh pr list` per distinct
// project directory (through that directory's most recent session); a row opens the same
// Summary / Changes / Checks view the session header's chip opens.
import { GitPullRequest } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { ActivityIndicator, RefreshControl, StyleSheet, Text, View } from "react-native";

import { PrDetailView } from "../components/pr/PrDetailView";
import { checksLabel, checksTone, projectReps } from "../components/pr/prModel";
import { toneColor } from "../components/pr/PrChip";
import { BackLink } from "../components/ds/BackLink";
import { Badge } from "../components/ds/Badge";
import { Chip } from "../components/ds/Chip";
import { EmptyState } from "../components/ds/EmptyState";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { Sheet } from "../components/ds/Sheet";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import type { PrListRow, PrState } from "../lib/apiPr";
import { useSessions } from "../lib/queries";
import { useAllPrs, usePrAvailable } from "../lib/queriesPr";
import { usePullRefresh } from "../lib/usePullRefresh";
import { SettingsShell } from "./(tabs)/settings";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { formatRelativeTime, type as typeScale } from "../theme/typography";

const STATES: { value: PrState; label: string }[] = [
  { value: "open", label: "Open" },
  { value: "merged", label: "Merged" },
  { value: "closed", label: "Closed" },
  { value: "all", label: "All" },
];

const ago = (iso: string): string => (Number.isNaN(Date.parse(iso)) ? "" : formatRelativeTime(Date.parse(iso)));

interface Row extends PrListRow {
  sessionId: string;
}

function PrsBody() {
  const tokens = useTokens();
  const direct = usePrAvailable();
  const sessions = useSessions();
  const [state, setState] = useState<PrState>("open");
  const [selected, setSelected] = useState<Row | null>(null);
  const reps = useMemo(() => projectReps(sessions.data ?? []), [sessions.data]);
  const results = useAllPrs(
    reps.map((r) => r.sessionId),
    state,
  );
  const pull = usePullRefresh(async () => {
    await Promise.all(results.map((r) => r.refetch()));
  });

  const rows = useMemo(() => {
    const seen = new Set<string>();
    const out: Row[] = [];
    results.forEach((result, i) => {
      for (const pr of result.data?.prs ?? []) {
        if (seen.has(pr.url)) continue;
        seen.add(pr.url);
        out.push({ ...pr, sessionId: reps[i].sessionId });
      }
    });
    return out.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  }, [results, reps]);

  const loading = results.some((r) => r.isPending) || sessions.isPending;
  const unavailable = results.find((r) => r.data && !r.data.available)?.data?.error ?? null;
  const groups = useMemo(() => {
    const map = new Map<string, Row[]>();
    for (const row of rows) map.set(row.repo || "other", [...(map.get(row.repo || "other") ?? []), row]);
    return [...map.entries()];
  }, [rows]);

  return (
    <Screen
      scroll
      refreshControl={<RefreshControl refreshing={pull.refreshing} onRefresh={pull.onRefresh} />}
      contentContainerStyle={styles.content}
    >
      <BackLink />
      <Text style={[typeScale.title, { color: tokens.ink }]}>Pull requests</Text>
      <View style={styles.chips}>
        {STATES.map((s) => (
          <Chip key={s.value} label={s.label} size="compact" selected={state === s.value} onPress={() => setState(s.value)} />
        ))}
      </View>
      {!direct ? (
        <EmptyState icon={GitPullRequest} message="Pull requests need a direct connection to the host." />
      ) : unavailable && rows.length === 0 ? (
        <EmptyState icon={GitPullRequest} message={unavailable} />
      ) : loading && rows.length === 0 ? (
        <ActivityIndicator color={tokens.accent} style={styles.loading} />
      ) : rows.length === 0 ? (
        <EmptyState icon={GitPullRequest} message={`No ${state === "all" ? "" : `${state} `}pull requests in your projects.`} />
      ) : (
        groups.map(([repo, list]) => (
          <View key={repo} style={styles.group}>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{repo}</Text>
            {list.map((pr, i) => (
              <ListRow
                key={pr.url}
                title={pr.title}
                subtitle={`#${pr.number} ${pr.head_ref} - ${pr.author} - ${ago(pr.updated_at)}`}
                trailing={
                  pr.state === "open" ? (
                    <Text style={[typeScale.monoMeta, { color: toneColor(checksTone(pr.checks), tokens) }]}>
                      {pr.draft ? "draft" : checksLabel(pr.checks)}
                    </Text>
                  ) : (
                    <Badge label={pr.state} tone={pr.state === "merged" ? "accent" : "neutral"} lowercase />
                  )
                }
                onPress={() => setSelected(pr)}
                showSeparator={i < list.length - 1}
                accessibilityLabel={`Pull request ${pr.number}: ${pr.title}`}
              />
            ))}
          </View>
        ))
      )}
      <Sheet visible={selected != null} onClose={() => setSelected(null)} accessibilityLabel="Pull request" snapPoints={[0.92]}>
        {selected ? <PrDetailView sessionId={selected.sessionId} number={selected.number} /> : null}
      </Sheet>
    </Screen>
  );
}

export default function PrsScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="prs">
        <PrsBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12, width: "100%", maxWidth: 1000, alignSelf: "center" },
  chips: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
  loading: { padding: space.space32 },
  group: { gap: space.space4 },
});
