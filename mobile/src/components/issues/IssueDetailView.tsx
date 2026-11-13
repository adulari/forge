import { CircleDot, ExternalLink } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { PrWatchMethod, PrWatchMode } from "../../lib/apiPr";
import { openSession } from "../../lib/nav";
import { useIssueDetail, useStartIssue } from "../../lib/queriesIssues";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { Segmented } from "../ds/Segmented";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";
import { WATCH_METHODS, WATCH_MODES } from "../pr/prWatchModel";
import { startHint, startRequest, type IssueEntry } from "./issuesModel";

function openUrl(url: string) {
  void Linking.openURL(url).catch(() => undefined);
}

export function IssueDetailView({ issue, onStarted }: { issue: IssueEntry; onStarted: () => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const detail = useIssueDetail(issue.sessionId, issue.number);
  const start = useStartIssue();
  const [worktree, setWorktree] = useState(true);
  const [mode, setMode] = useState<PrWatchMode>("fix");
  const [method, setMethod] = useState<PrWatchMethod>("squash");

  const begin = () =>
    start.mutate(startRequest(issue, { worktree, mode, method }), {
      onSuccess: (started) => {
        onStarted();
        openSession(started.id);
      },
      onError: (e) => toast.show(e.message, { tone: "danger" }),
    });

  return (
    <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.scroll}>
      <View style={styles.head}>
        <View style={styles.titleRow}>
          <Text style={[typeScale.heading, styles.flex, { color: tokens.ink }]} numberOfLines={3}>
            {issue.title}
          </Text>
          <Pressable
            onPress={() => openUrl(issue.url)}
            accessibilityRole="link"
            accessibilityLabel="Open on GitHub"
            hitSlop={8}
          >
            <ExternalLink size={16} color={tokens.ink3} />
          </Pressable>
        </View>
        <View style={styles.metaRow}>
          <Badge label={issue.state} tone={issue.state === "open" ? "success" : "neutral"} />
          {issue.labels.map((label) => (
            <Badge key={label.name} label={label.name} tone="neutral" lowercase />
          ))}
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
            #{issue.number} {issue.author}
          </Text>
        </View>
      </View>

      <View style={styles.body}>
        {issue.state === "open" ? (
          <View style={[styles.panel, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Start a session</Text>
            <View style={styles.switchRow}>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Work in a new worktree</Text>
              <Switch value={worktree} onValueChange={setWorktree} accessibilityLabel="Work in a new worktree" />
            </View>
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>PR Watch</Text>
            <Segmented options={WATCH_MODES} value={mode} onChange={setMode} />
            {mode === "merge" ? <Segmented options={WATCH_METHODS} value={method} onChange={setMethod} /> : null}
            <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{startHint(mode, worktree)}</Text>
            <Button label="Start session" onPress={begin} loading={start.isPending} fullWidth />
          </View>
        ) : null}

        {detail.isPending ? (
          <ActivityIndicator color={tokens.accent} style={styles.loading} />
        ) : detail.error || !detail.data ? (
          <EmptyState icon={CircleDot} message={detail.error?.message ?? "Could not load the issue."} />
        ) : (
          <>
            {detail.data.body.trim() ? (
              <Text style={[typeScale.body, { color: tokens.ink2 }]} selectable>
                {detail.data.body.trim()}
              </Text>
            ) : (
              <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No description.</Text>
            )}
            <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Comments ({detail.data.comments.length})</Text>
            {detail.data.comments.map((c, i) => (
              <View key={`${c.created_at}-${i}`} style={[styles.comment, { borderColor: tokens.border }]}>
                <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{c.author}</Text>
                <Text style={[typeScale.sub, { color: tokens.ink }]} selectable>
                  {c.body}
                </Text>
              </View>
            ))}
          </>
        )}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  scroll: { paddingBottom: space.space24 },
  loading: { padding: space.space32 },
  head: {
    paddingHorizontal: space.space16,
    paddingBottom: space.space8,
    gap: space.space8,
  },
  titleRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: space.space8,
  },
  metaRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    flexWrap: "wrap",
  },
  body: {
    paddingHorizontal: space.space16,
    paddingTop: space.space8,
    gap: space.space8,
  },
  panel: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius4,
    padding: space.space12,
    gap: space.space8,
  },
  switchRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  comment: { borderLeftWidth: 2, paddingLeft: space.space8, gap: space.space2 },
});
