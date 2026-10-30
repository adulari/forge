import { ExternalLink, GitBranch } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { MergeMethod, PrDetailResponse } from "../../lib/apiPr";
import { useCommentOnPr, useMergePr, usePrDetail, usePrDiff } from "../../lib/queriesPr";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";
import { Switch } from "../ds/Switch";
import { TabStrip } from "../ds/TabStrip";
import { useToast } from "../ds/ToastHost";
import { toneColor } from "./PrChip";
import { PrFileCard } from "./PrFileCard";
import { PrWatchPanel } from "./PrWatchPanel";
import { checksLabel, checksTone, mergeGate } from "./prModel";

type Tab = "summary" | "changes" | "checks";

const METHODS: { value: MergeMethod; label: string }[] = [
  { value: "squash", label: "Squash" },
  { value: "merge", label: "Merge" },
  { value: "rebase", label: "Rebase" },
];

const SYNC_COPY = {
  pulled: "Base branch pulled",
  fetched: "Base branch fetched",
  skipped: "",
} as const;

function openUrl(url: string) {
  void Linking.openURL(url).catch(() => undefined);
}

function stateTone(state: string): "success" | "accent" | "neutral" {
  return state === "open" ? "success" : state === "merged" ? "accent" : "neutral";
}

function SummaryTab({ sessionId, detail }: { sessionId: string; detail: PrDetailResponse }) {
  const tokens = useTokens();
  const toast = useToast();
  const comment = useCommentOnPr();
  const merge = useMergePr();
  const [draft, setDraft] = useState("");
  const [method, setMethod] = useState<MergeMethod>("squash");
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const { pr } = detail;
  const gate = mergeGate(pr);

  const send = () => {
    const body = draft.trim();
    if (!body) return;
    comment.mutate(
      { session: sessionId, number: pr.number, body },
      {
        onSuccess: () => setDraft(""),
        onError: (e) => toast.show(e.message, { tone: "danger" }),
      },
    );
  };
  const doMerge = () => {
    setConfirming(false);
    merge.mutate(
      { session: sessionId, number: pr.number, head_sha: pr.head_sha, method, delete_branch: deleteBranch },
      {
        onSuccess: (result) => {
          const sync = SYNC_COPY[result.sync];
          toast.show(sync ? `Merged #${pr.number}. ${sync}.` : `Merged #${pr.number}`, { tone: "success" });
        },
        onError: (e) => toast.show(e.message, { tone: "danger" }),
      },
    );
  };

  return (
    <View style={styles.tabBody}>
      {detail.body.trim() ? (
        <Text style={[typeScale.body, { color: tokens.ink2 }]} selectable>
          {detail.body.trim()}
        </Text>
      ) : (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No description.</Text>
      )}

      {pr.state === "open" ? <PrWatchPanel sessionId={sessionId} /> : null}

      {pr.state === "open" ? (
        <View style={[styles.panel, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
          <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Merge</Text>
          <Segmented options={METHODS} value={method} onChange={setMethod} />
          <View style={styles.switchRow}>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Delete branch after merge</Text>
            <Switch value={deleteBranch} onValueChange={setDeleteBranch} accessibilityLabel="Delete branch after merge" />
          </View>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>
            Binds to commit {pr.head_sha.slice(0, 7)}; a newer push is refused.
          </Text>
          {gate.reason ? <Text style={[typeScale.monoMeta, { color: gate.allowed ? tokens.warn : tokens.danger }]}>{gate.reason}</Text> : null}
          <Button
            label={`Merge #${pr.number}`}
            onPress={() => setConfirming(true)}
            disabled={!gate.allowed}
            loading={merge.isPending}
            fullWidth
          />
        </View>
      ) : null}

      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Commits ({detail.commits.length})</Text>
      {detail.commits.map((c) => (
        <View key={c.sha} style={styles.commitRow}>
          <Text style={[styles.sha, tabularNums, { color: tokens.ink3 }]}>{c.sha.slice(0, 7)}</Text>
          <Text style={[typeScale.sub, styles.flex, { color: tokens.ink }]} numberOfLines={1}>
            {c.headline}
          </Text>
        </View>
      ))}

      <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Comments ({detail.comments.length})</Text>
      {detail.comments.map((c, i) => (
        <View key={`${c.created_at}-${i}`} style={[styles.comment, { borderColor: tokens.border }]}>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{c.author}</Text>
          <Text style={[typeScale.sub, { color: tokens.ink }]} selectable>
            {c.body}
          </Text>
        </View>
      ))}
      <Input
        label="Add a comment"
        value={draft}
        onChangeText={setDraft}
        multiline
        placeholder="Leave a comment"
        accessibilityLabel="Pull request comment"
      />
      <Button label="Comment" variant="secondary" onPress={send} disabled={!draft.trim()} loading={comment.isPending} />

      <ConfirmDialog
        visible={confirming}
        title={`Merge #${pr.number}?`}
        message={`${method === "squash" ? "Squash and merge" : method === "rebase" ? "Rebase and merge" : "Create a merge commit for"} ${pr.head_ref} into ${pr.base_ref}.`}
        confirmLabel="Merge"
        onConfirm={doMerge}
        onCancel={() => setConfirming(false)}
      />
    </View>
  );
}

function ChangesTab({ sessionId, number, commits }: { sessionId: string; number: number; commits: PrDetailResponse["commits"] }) {
  const tokens = useTokens();
  const [hideWhitespace, setHideWhitespace] = useState(false);
  const [commit, setCommit] = useState<string | null>(null);
  const diff = usePrDiff(sessionId, number, commit, hideWhitespace, true);
  const files = diff.data?.files ?? [];
  return (
    <View style={styles.tabBody}>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.chipRow}>
        <Chip label="Hide whitespace" size="compact" selected={hideWhitespace} onPress={() => setHideWhitespace((v) => !v)} />
        <Chip label="All commits" size="compact" selected={commit === null} onPress={() => setCommit(null)} />
        {commits.map((c) => (
          <Chip
            key={c.sha}
            label={c.sha.slice(0, 7)}
            size="compact"
            selected={commit === c.sha}
            onPress={() => setCommit(commit === c.sha ? null : c.sha)}
          />
        ))}
      </ScrollView>
      {diff.isPending ? (
        <ActivityIndicator color={tokens.accent} />
      ) : diff.error ? (
        <Text style={[typeScale.sub, { color: tokens.danger }]}>{diff.error.message}</Text>
      ) : files.length === 0 ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          {hideWhitespace ? "Only whitespace changed." : "No changes."}
        </Text>
      ) : (
        files.map((file) => <PrFileCard key={`${commit}-${file.path}`} file={file} defaultOpen={files.length <= 6} />)
      )}
    </View>
  );
}

function ChecksTab({ detail }: { detail: PrDetailResponse }) {
  const tokens = useTokens();
  const { checks } = detail.pr;
  if (checks.items.length === 0) {
    return <Text style={[typeScale.sub, styles.tabBody, { color: tokens.ink3 }]}>No checks reported for this pull request.</Text>;
  }
  return (
    <View style={styles.tabBody}>
      {checks.items.map((item, i) => (
        <Pressable
          key={`${item.name}-${i}`}
          onPress={item.url ? () => openUrl(item.url as string) : undefined}
          accessibilityRole={item.url ? "link" : "text"}
          accessibilityLabel={`${item.name}: ${item.detail}`}
          style={styles.checkRow}
        >
          <View style={[styles.dot, { backgroundColor: toneColor(item.bucket === "passing" ? "success" : item.bucket === "failing" ? "danger" : "warn", tokens) }]} />
          <View style={styles.flex}>
            <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
              {item.name}
            </Text>
            {item.workflow ? (
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                {item.workflow}
              </Text>
            ) : null}
          </View>
          <Text style={[typeScale.monoMeta, { color: toneColor(item.bucket === "passing" ? "success" : item.bucket === "failing" ? "danger" : "warn", tokens) }]}>
            {item.detail}
          </Text>
        </Pressable>
      ))}
    </View>
  );
}

export function PrDetailView({ sessionId, number }: { sessionId: string; number: number }) {
  const tokens = useTokens();
  const [tab, setTab] = useState<Tab>("summary");
  const detail = usePrDetail(sessionId, number);
  if (detail.isPending) {
    return <ActivityIndicator color={tokens.accent} style={styles.loading} />;
  }
  if (detail.error || !detail.data) {
    return <EmptyState icon={GitBranch} message={detail.error?.message ?? "Could not load the pull request."} />;
  }
  const { pr } = detail.data;
  return (
    <View style={styles.root}>
      <View style={styles.head}>
        <View style={styles.titleRow}>
          <Text style={[typeScale.heading, styles.flex, { color: tokens.ink }]} numberOfLines={2}>
            {pr.title}
          </Text>
          <Pressable onPress={() => openUrl(pr.url)} accessibilityRole="link" accessibilityLabel="Open on GitHub" hitSlop={8}>
            <ExternalLink size={16} color={tokens.ink3} />
          </Pressable>
        </View>
        <View style={styles.metaRow}>
          <Badge label={pr.draft ? "draft" : pr.state} tone={pr.draft ? "neutral" : stateTone(pr.state)} />
          <Text style={[typeScale.monoMeta, styles.flex, { color: tokens.ink3 }]} numberOfLines={1}>
            #{pr.number} {pr.head_ref} {"->"} {pr.base_ref}
          </Text>
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]}>+{pr.additions}</Text>
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.danger }]}>-{pr.deletions}</Text>
        </View>
      </View>
      <TabStrip<Tab>
        options={[
          { value: "summary", label: "Summary" },
          { value: "changes", label: "Changes", badge: pr.changed_files },
          { value: "checks", label: "Checks", badge: pr.checks.total, dot: pr.checks.failing > 0 },
        ]}
        value={tab}
        onChange={setTab}
      />
      <Text style={[typeScale.monoMeta, styles.checksLine, { color: toneColor(checksTone(pr.checks), tokens) }]}>
        {checksLabel(pr.checks)}
      </Text>
      <ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.scroll}>
        {tab === "summary" ? <SummaryTab sessionId={sessionId} detail={detail.data} /> : null}
        {tab === "changes" ? <ChangesTab sessionId={sessionId} number={number} commits={detail.data.commits} /> : null}
        {tab === "checks" ? <ChecksTab detail={detail.data} /> : null}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, minHeight: 0 },
  flex: { flex: 1 },
  loading: { padding: space.space32 },
  head: { paddingHorizontal: space.space16, paddingBottom: space.space8, gap: space.space8 },
  titleRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  metaRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  checksLine: { paddingHorizontal: space.space16, paddingTop: space.space8, fontFamily: monoFamily.regular },
  scroll: { paddingBottom: space.space24 },
  tabBody: { paddingHorizontal: space.space16, paddingTop: space.space12, gap: space.space8 },
  panel: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12, gap: space.space8 },
  switchRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  commitRow: { flexDirection: "row", gap: space.space8, alignItems: "center" },
  sha: { fontFamily: monoFamily.regular, fontSize: 11.5 },
  comment: { borderLeftWidth: 2, paddingLeft: space.space8, gap: space.space2 },
  chipRow: { gap: space.space8, paddingBottom: space.space4 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  checkRow: { flexDirection: "row", alignItems: "center", gap: space.space12, minHeight: 40 },
});
