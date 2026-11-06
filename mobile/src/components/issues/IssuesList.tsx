// Issues across every project the host runs sessions in, next to the pull requests. One
// `gh issue list` per distinct project directory; a row opens the issue with its Start action.
import { CircleDot } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import type { IssueState } from "../../lib/apiIssues";
import { useAllIssues } from "../../lib/queriesIssues";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatRelativeTime, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Chip } from "../ds/Chip";
import { EmptyState } from "../ds/EmptyState";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { IssueDetailView } from "./IssueDetailView";
import { issueSubtitle, mergeIssues, type IssueEntry } from "./issuesModel";

const STATES: { value: IssueState; label: string }[] = [
  { value: "open", label: "Open" },
  { value: "closed", label: "Closed" },
  { value: "all", label: "All" },
];

const ago = (iso: string): string => (Number.isNaN(Date.parse(iso)) ? "" : formatRelativeTime(Date.parse(iso)));

export function IssuesList({ sessionIds }: { sessionIds: readonly string[] }) {
  const tokens = useTokens();
  const [state, setState] = useState<IssueState>("open");
  const [selected, setSelected] = useState<IssueEntry | null>(null);
  const results = useAllIssues(sessionIds, state);
  const issues = mergeIssues(
    results.map((r) => r.data),
    sessionIds,
  );
  const loading = results.some((r) => r.isPending);
  const unavailable = results.find((r) => r.data && !r.data.available)?.data?.error ?? null;
  const byRepo = new Map<string, IssueEntry[]>();
  for (const issue of issues) byRepo.set(issue.repo || "other", [...(byRepo.get(issue.repo || "other") ?? []), issue]);
  const groups = [...byRepo.entries()];

  return (
    <>
      <View style={styles.chips}>
        {STATES.map((s) => (
          <Chip
            key={s.value}
            label={s.label}
            size="compact"
            selected={state === s.value}
            onPress={() => setState(s.value)}
          />
        ))}
      </View>
      {unavailable && issues.length === 0 ? (
        <EmptyState icon={CircleDot} message={unavailable} />
      ) : loading && issues.length === 0 ? (
        <ActivityIndicator color={tokens.accent} style={styles.loading} />
      ) : issues.length === 0 ? (
        <EmptyState icon={CircleDot} message={`No ${state === "all" ? "" : `${state} `}issues in your projects.`} />
      ) : (
        groups.map(([repo, list]) => (
          <View key={repo} style={styles.group}>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{repo}</Text>
            {list.map((issue, i) => (
              <ListRow
                key={issue.url}
                title={issue.title}
                subtitle={issueSubtitle(issue, ago(issue.updated_at))}
                trailing={
                  issue.labels.length > 0 ? (
                    <Badge label={issue.labels[0].name} tone="neutral" lowercase />
                  ) : issue.state === "closed" ? (
                    <Badge label="closed" tone="neutral" lowercase />
                  ) : undefined
                }
                onPress={() => setSelected(issue)}
                showSeparator={i < list.length - 1}
                accessibilityLabel={`Issue ${issue.number}: ${issue.title}`}
              />
            ))}
          </View>
        ))
      )}
      <Sheet
        visible={selected != null}
        onClose={() => setSelected(null)}
        accessibilityLabel="Issue"
        snapPoints={[0.92]}
      >
        {selected ? <IssueDetailView issue={selected} onStarted={() => setSelected(null)} /> : null}
      </Sheet>
    </>
  );
}

const styles = StyleSheet.create({
  chips: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
  loading: { padding: space.space32 },
  group: { gap: space.space4 },
});
