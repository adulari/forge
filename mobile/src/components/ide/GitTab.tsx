// Branch, sync, the staged and unstaged lists and the commit box for the agent in view: the
// Review dock's left column, reused as is. Picking a file opens its diff in the editor.
import { GitBranch, Rocket } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { StyleSheet, Text, View , ScrollView } from "react-native";

import { useAuth } from "../../lib/auth";
import { openShip } from "../../lib/shipRequest";
import { useProjectActions } from "../../lib/queriesProjectActions";
import { useCommitAction } from "../../lib/commitAction";
import { useDiscardPaths, useGenerateCommitMessage, usePushGit } from "../../lib/gitActions";
import { useCommitStaged, useGitStatus, useStagePaths, useUnstagePaths } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { EmptyState } from "../ds/EmptyState";
import { useToast } from "../ds/ToastHost";
import { GitBranchPicker } from "../git/GitBranchPicker";
import { GitCommitBox } from "../git/GitCommitBox";
import { GitFileList, type GitSelection } from "../git/GitFileList";
import { GitSyncBar } from "../git/GitSyncBar";
import { type CommitAction, type DiscardKind, classifyGitStatusError, discardMessage, isGitReviewReadOnly, isGitReviewSupported } from "../git/gitReviewSupport";
import { useIde } from "./IdeContext";
import { IdeTextButton } from "./ideUi";
import { PullRequestSheet } from "../session/PullRequestSheet";
import { usePrAvailable, usePrStatus } from "../../lib/queriesPr";
import { checkActions, checkState } from "./checksModel";
import { failedChecksWarning, postShipAction, shipHint, shipLabel, shipSteps, type ShipStep } from "./shipModel";

export function GitTab() {
  const { panelSessionId } = useIde();
  if (!panelSessionId) return <EmptyState icon={GitBranch} message="Pick an agent to see its branch and working tree." />;
  return <GitColumn key={panelSessionId} sessionId={panelSessionId} />;
}

function GitColumn({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl, host } = useAuth();
  const { openChange, active } = useIde();
  const supported = isGitReviewSupported(baseUrl);
  const readOnly = isGitReviewReadOnly(baseUrl);
  const status = useGitStatus(supported ? sessionId : null);
  const stage = useStagePaths();
  const unstage = useUnstagePaths();
  const commit = useCommitStaged();
  const push = usePushGit();
  const discard = useDiscardPaths();
  const generate = useGenerateCommitMessage();
  const [message, setMessage] = useState("");
  const [pushedTo, setPushedTo] = useState<string | null>(null);
  const [commitAction, setCommitAction] = useCommitAction();
  const [manual, setManual] = useState(false);
  const [shipping, setShipping] = useState<ShipStep | null>(null);
  const [prVisible, setPrVisible] = useState(false);
  const prDirect = usePrAvailable();
  const prStatus = usePrStatus(pushedTo && prDirect ? sessionId : null);
  const actionsQuery = useProjectActions(sessionId);
  const [pending, setPending] = useState<{ paths: string[]; kind: DiscardKind } | null>(null);

  const select = useCallback((selection: GitSelection) => openChange(sessionId, selection.path), [openChange, sessionId]);
  const selected: GitSelection | null = active?.kind === "diff" && active.sessionId === sessionId ? { path: active.path, staged: false } : null;

  const data = status.data;
  const problem = status.error ? classifyGitStatusError(status.error, host) : null;
  if (problem && problem.kind !== "error") return <EmptyState icon={GitBranch} message={problem.message} />;
  if (status.error) return <Text style={[typeScale.sub, styles.notice, { color: tokens.danger }]}>{status.error.message}</Text>;
  if (!data) return <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>Loading working tree…</Text>;

  const clean = data.staged.length + data.unstaged.length + data.untracked.length + (data.conflicted?.length ?? 0) === 0;

  const run = async (action: CommitAction) => {
    setPushedTo(null);
    push.reset();
    try {
      await commit.mutateAsync({ session: sessionId, message });
    } catch {
      return;
    }
    setMessage("");
    if (action !== "commit") {
      try {
        const pushed = await push.mutateAsync({ session: sessionId });
        setPushedTo(pushed.remote ?? "remote");
      } catch {
        // The push error renders under the commit result; the commit itself stands.
      }
    }
  };

  const unstagedPaths = [...data.unstaged, ...data.untracked].map((row) => row.path);
  const postShip = postShipAction(pushedTo != null, prStatus.data?.pr);
  const failedChecks = failedChecksWarning(checkActions(actionsQuery.data?.actions ?? []).filter((action) => checkState(action) === "fail").map((action) => action.name));
  const steps = shipSteps({ staged: data.staged.length, unstaged: unstagedPaths.length, hasMessage: message.trim().length > 0, canPush: data.has_remote !== false });
  const ship = async () => {
    setPushedTo(null);
    push.reset();
    commit.reset();
    let text = message;
    try {
      for (const step of steps) {
        setShipping(step);
        if (step === "stage") await stage.mutateAsync({ session: sessionId, paths: unstagedPaths });
        else if (step === "message") {
          text = (await generate.mutateAsync({ session: sessionId })).message;
          setMessage(text);
        } else if (step === "commit") {
          await commit.mutateAsync({ session: sessionId, message: text });
          setMessage("");
        } else {
          const pushed = await push.mutateAsync({ session: sessionId });
          setPushedTo(pushed.remote ?? "remote");
        }
      }
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not finish shipping.", { tone: "danger" });
    } finally {
      setShipping(null);
    }
  };

  const confirmDiscard = async () => {
    const paths = pending?.paths;
    setPending(null);
    if (!paths) return;
    try {
      const result = await discard.mutateAsync({ session: sessionId, paths });
      toast.show(`Discarded changes to ${result.discarded} file${result.discarded === 1 ? "" : "s"}`, { tone: "neutral" });
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not discard.", { tone: "danger" });
    }
  };
  const copy = pending ? discardMessage(pending.paths, pending.kind) : null;

  return (
    <View style={styles.fill}>
      <GitBranchPicker sessionId={sessionId} branch={data.branch} baseBranch={data.base_branch} readOnly={readOnly} />
      {!readOnly ? <GitSyncBar sessionId={sessionId} status={data} /> : null}
      <View style={styles.fill}>
        {clean ? (
          <EmptyState icon={GitBranch} message="Working tree clean. Nothing to commit." />
        ) : (
          <ScrollView>
            <GitFileList
              status={data}
              selected={selected}
              onSelect={select}
              onStage={readOnly ? null : (paths) => stage.mutate({ session: sessionId, paths })}
              onUnstage={readOnly ? null : (paths) => unstage.mutate({ session: sessionId, paths })}
              onDiscard={readOnly ? null : (paths, kind) => setPending({ paths, kind })}
              onResolveConflicts={null}
              busy={stage.isPending || unstage.isPending || discard.isPending}
            />
          </ScrollView>
        )}
      </View>
      {!readOnly ? (
        <View style={[styles.ship, { borderTopColor: tokens.border }]}>
          <View style={styles.shipText}>
            <Text numberOfLines={2} style={[typeScale.meta, { color: shipping ? tokens.ink : tokens.ink3 }]}>
              {shipping ? `Now: ${shipping === "message" ? "writing a message" : shipping === "stage" ? "staging" : shipping === "commit" ? "committing" : "pushing"}…` : pushedTo ? `Shipped to ${pushedTo}.` : commit.data ? `Committed ${commit.data.sha.slice(0, 7)}.` : shipHint(steps)}
            </Text>
            {failedChecks && !shipping ? <Text numberOfLines={1} style={[typeScale.meta, { color: tokens.danger }]}>{failedChecks}</Text> : null}
            {message.trim() ? (
              <Text numberOfLines={1} style={[typeScale.monoMeta, { color: tokens.ink2 }]}>
                {message.split("\n")[0]}
              </Text>
            ) : null}
          </View>
          <View style={styles.shipButtons}>
            {postShip.kind !== "none" && !shipping ? (
              <IdeTextButton label={postShip.kind === "view-pr" ? `View PR #${postShip.number}` : "Open pull request"} onPress={() => setPrVisible(true)} />
            ) : null}
            {prDirect ? <IdeTextButton label="Ship to PR" onPress={() => openShip(sessionId)} /> : null}
            <IdeTextButton label={manual ? "Hide message" : "Edit message"} onPress={() => setManual((value) => !value)} />
            <IdeTextButton
              label={shipLabel(steps)}
              tone="accent"
              icon={<Rocket size={12} strokeWidth={1.75} color={tokens.accent} />}
              disabled={steps.length === 0}
              loading={shipping != null}
              onPress={() => void ship()}
            />
          </View>
        </View>
      ) : null}
      {!readOnly && manual ? (
        <GitCommitBox
          branch={data.branch}
          baseBranch={data.base_branch}
          stagedCount={data.staged.length}
          message={message}
          onChangeMessage={setMessage}
          onCommit={(action) => void run(action)}
          committing={commit.isPending}
          pushing={push.isPending}
          canPush={data.has_remote !== false}
          action={commitAction}
          onChooseAction={setCommitAction}
          onGenerate={() => {
            void generate
              .mutateAsync({ session: sessionId })
              .then((generated) => setMessage(generated.message))
              .catch((error: unknown) => toast.show(error instanceof Error ? error.message : "Could not generate a commit message.", { tone: "danger" }));
          }}
          generating={generate.isPending}
          error={commit.error ?? push.error}
          result={commit.data ?? null}
          pushedTo={pushedTo}
        />
      ) : null}
      <PullRequestSheet visible={prVisible} onClose={() => setPrVisible(false)} sessionId={sessionId} />
      <ConfirmDialog
        visible={pending != null}
        title={copy?.title ?? "Discard changes?"}
        message={copy?.message}
        confirmLabel={pending?.kind === "untracked" ? "Delete" : "Discard"}
        destructive
        loading={discard.isPending}
        onConfirm={() => void confirmDiscard()}
        onCancel={() => setPending(null)}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0 },
  ship: { gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8, borderTopWidth: StyleSheet.hairlineWidth },
  shipButtons: { flexDirection: "row", alignItems: "center", justifyContent: "flex-end", gap: space.space8 },
  shipText: { minWidth: 0, gap: 2 },
  notice: { paddingHorizontal: space.space12, paddingVertical: space.space16 },
});
