// The merge queue across repos: who lands next, what the queue did to a branch, and the owner's
// hold / release / move-to-front / retry on each row. Rows come from the daemon already ordered
// (waiting by position, then held and handed-back, then recent merges and blocks).
import { GitMerge } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, Linking, Pressable, StyleSheet, Text, View } from "react-native";

import type { PrQueueAction, PrQueueRow } from "../../lib/apiPr";
import { usePrAvailable, usePrQueue, useQueueAction } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { Sheet } from "../ds/Sheet";
import { PrDetailView } from "./PrDetailView";
import { ACTION_LABELS, availableActions, pushesLabel, repoName, rowStatus } from "./mergeQueueModel";

interface Pending {
  session: string;
  action: PrQueueAction;
}

export function MergeQueueView() {
  const tokens = useTokens();
  const direct = usePrAvailable();
  const query = usePrQueue();
  const act = useQueueAction();
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<{ session: string; message: string } | null>(null);
  const [opened, setOpened] = useState<PrQueueRow | null>(null);

  const run = (row: PrQueueRow, action: PrQueueAction) => {
    setPending({ session: row.session_id, action });
    setError(null);
    act.mutate(
      { session: row.session_id, action },
      {
        onError: (e) => setError({ session: row.session_id, message: e.message }),
        onSettled: () => setPending(null),
      },
    );
  };

  const openPr = (row: PrQueueRow) => {
    if (direct && row.pr_number != null) setOpened(row);
    else if (row.pr_url) void Linking.openURL(row.pr_url).catch(() => undefined);
  };

  const queues = (query.data ?? []).filter((queue) => queue.rows.length > 0);

  if (query.isPending) return <ActivityIndicator color={tokens.accent} style={styles.loading} />;
  if (query.isError && queues.length === 0) {
    return <EmptyState icon={GitMerge} message={`Could not read the merge queue: ${query.error.message}`} />;
  }
  if (queues.length === 0) {
    return (
      <EmptyState
        icon={GitMerge}
        message="The merge queue is empty. Arm PR Watch in Fix + merge mode on a session (Ship does it for you), or start an issue drain, and its pull request lines up here."
      />
    );
  }

  return (
    <View style={styles.root}>
      {queues.map((queue) => (
        <View key={queue.repo} style={styles.group}>
          <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{repoName(queue.repo)}</Text>
          {queue.rows.map((row) => {
            const status = rowStatus(row);
            const actions = availableActions(row);
            const extra = pushesLabel(row);
            const mine = error?.session === row.session_id ? error.message : null;
            return (
              <View key={row.session_id} style={[styles.row, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
                <View style={styles.head}>
                  <Text style={[typeScale.monoMeta, styles.position, { color: tokens.ink3 }]}>
                    {row.position != null ? `${row.position}` : "-"}
                  </Text>
                  <Pressable
                    onPress={() => openPr(row)}
                    accessibilityRole="link"
                    accessibilityLabel={row.pr_number != null ? `Open pull request ${row.pr_number}` : "Open pull request"}
                    style={styles.flex}
                  >
                    <Text style={[typeScale.body, { color: tokens.accent }]}>
                      {row.pr_number != null ? `PR #${row.pr_number}` : "Pull request pending"}
                    </Text>
                  </Pressable>
                  <Badge label={status.label} tone={status.tone} />
                </View>
                {status.detail ? (
                  <Text style={[typeScale.monoMeta, { color: status.tone === "danger" ? tokens.danger : tokens.ink3 }]}>{status.detail}</Text>
                ) : null}
                {extra ? <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{extra}</Text> : null}
                {actions.length > 0 ? (
                  <View style={styles.actions}>
                    {actions.map((action) => (
                      <Button
                        key={action}
                        label={ACTION_LABELS[action]}
                        variant="secondary"
                        loading={pending?.session === row.session_id && pending.action === action}
                        disabled={pending != null}
                        onPress={() => run(row, action)}
                        accessibilityLabel={`${ACTION_LABELS[action]} ${row.pr_number != null ? `pull request ${row.pr_number}` : "this pull request"}`}
                      />
                    ))}
                  </View>
                ) : null}
                {mine ? (
                  <Text accessibilityRole="alert" style={[typeScale.monoMeta, { color: tokens.danger }]}>
                    {mine}
                  </Text>
                ) : null}
              </View>
            );
          })}
        </View>
      ))}
      <Sheet visible={opened != null} onClose={() => setOpened(null)} accessibilityLabel="Pull request" snapPoints={[0.92]}>
        {opened && opened.pr_number != null ? <PrDetailView sessionId={opened.session_id} number={opened.pr_number} /> : null}
      </Sheet>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { gap: space.space16 },
  loading: { padding: space.space32 },
  group: { gap: space.space8 },
  row: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  position: { width: 20 },
  flex: { flex: 1 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
