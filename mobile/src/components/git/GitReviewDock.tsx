// Machined git review dock (docs/design/machined "Forge Machined - Desktop.dc.html" L265-308,
// D Git Review; INVENTORY.md L76-77 — the staged/unstaged list + split-diff browser + commit
// box had no equivalent in the app before this).
//
// Desktop chrome, but it does not assume a 1440px window: the dock measures itself and stacks
// the file column above the diff pane when its container is narrow, and the diff drops to a
// unified view when two columns would be narrower than a code line.
//
// Every row, count, branch name and error string here comes from `/api/git/*` — the dock has
// no local model of the working tree, and a failed request surfaces the daemon's own text
// rather than a generic "something went wrong".
import { GitBranch } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { ConflictResolver, type ConflictResolveRequest } from "./ConflictResolver";
import { GitBranchPicker } from "./GitBranchPicker";
import { GitCommitBox } from "./GitCommitBox";
import { GitDiffPane } from "./GitDiffPane";
import { GitFileList, type GitSelection } from "./GitFileList";
import { GitSyncBar } from "./GitSyncBar";
import { buildDiffEntries, selectionKey } from "./gitDiffEntries";
import {
  type CommitAction,
  type DiscardKind,
  classifyGitStatusError,
  discardMessage,
  isGitReviewReadOnly,
  isGitReviewSupported,
} from "./gitReviewSupport";
import { type GitStatusResponse } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useCommitAction } from "../../lib/commitAction";
import { useDiscardPaths, useGenerateCommitMessage, usePushGit } from "../../lib/gitActions";
import { useCommitStaged, useGitStatus, useStagePaths, useUnstagePaths } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { Segmented } from "../ds/Segmented";
import { EmptyState } from "../ds/EmptyState";
import { useToast } from "../ds/ToastHost";

const FILE_COLUMN_WIDTH = 264;
/** Below this the 264px column plus a readable diff no longer fit side by side. */
const STACK_BELOW = 700;

type StackedView = "files" | "diff";

function firstSelection(status: GitStatusResponse): GitSelection | null {
  const staged = status.staged[0];
  if (staged) return { path: staged.path, staged: true };
  const unstaged = status.unstaged[0] ?? status.untracked[0];
  if (unstaged) return { path: unstaged.path, staged: false };
  return null;
}

export function GitReviewDock({ sessionId }: { sessionId: string }): React.JSX.Element {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl, host } = useAuth();
  const [width, setWidth] = useState(0);
  const [selection, setSelection] = useState<GitSelection | null>(null);
  const [focusRequest, setFocusRequest] = useState<{ key: string; nonce: number } | null>(null);
  const [message, setMessage] = useState("");
  const [ignoreWhitespace, setIgnoreWhitespace] = useState(false);
  const [pendingDiscard, setPendingDiscard] = useState<{ paths: string[]; kind: DiscardKind } | null>(null);
  const [resolving, setResolving] = useState<ConflictResolveRequest | null>(null);
  const [pushedTo, setPushedTo] = useState<string | null>(null);
  const [commitAction, setCommitAction] = useCommitAction();
  const [stackedView, setStackedView] = useState<StackedView>("files");

  // Every row here comes from `/api/git/*`. The bridge carries the READING routes (git_status,
  // git_branches, git_diff), so reviewing works on every transport; it deliberately does not carry
  // the mutating ones, which the host refuses. Over Anywhere the dock therefore drops its index
  // and commit controls rather than offering presses whose requests come back denied.
  const supported = isGitReviewSupported(baseUrl);
  const readOnly = isGitReviewReadOnly(baseUrl);

  const status = useGitStatus(supported ? sessionId : null);
  const stage = useStagePaths();
  const unstage = useUnstagePaths();
  const commit = useCommitStaged();
  const push = usePushGit();
  const discard = useDiscardPaths();
  const generate = useGenerateCommitMessage();

  const data = status.data;
  const entries = useMemo(() => (data ? buildDiffEntries(data) : []), [data]);

  // Staging moves a path between buckets, which would otherwise strand the selection on a
  // bucket that no longer holds it: follow the file across instead of clearing the list's
  // highlight.
  useEffect(() => {
    if (!data) return;
    const holds = (candidate: GitSelection) =>
      (candidate.staged ? data.staged : [...data.unstaged, ...data.untracked]).some(
        (row) => row.path === candidate.path,
      );
    setSelection((current) => {
      if (current && holds(current)) return current;
      if (current) {
        const flipped: GitSelection = { path: current.path, staged: !current.staged };
        if (holds(flipped)) return flipped;
      }
      return firstSelection(data);
    });
  }, [data]);

  const selectFile = useCallback((next: GitSelection) => {
    setSelection(next);
    setStackedView("diff");
    const key = selectionKey(next);
    if (key) setFocusRequest((current) => ({ key, nonce: (current?.nonce ?? 0) + 1 }));
  }, []);

  const runCommit = async (action: CommitAction) => {
    setPushedTo(null);
    push.reset();
    try {
      await commit.mutateAsync({ session: sessionId, message });
    } catch {
      return;
    }
    setMessage("");
    if (action === "commit-push") {
      try {
        const pushed = await push.mutateAsync({ session: sessionId });
        setPushedTo(pushed.remote ?? "remote");
      } catch {
        // The push error renders under the commit result; the commit itself stands.
      }
    }
  };

  const generateMessage = async () => {
    try {
      const generated = await generate.mutateAsync({ session: sessionId });
      setMessage(generated.message);
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not generate a commit message.", {
        tone: "danger",
      });
    }
  };

  const confirmDiscard = async () => {
    const paths = pendingDiscard?.paths;
    setPendingDiscard(null);
    if (!paths) return;
    try {
      const result = await discard.mutateAsync({ session: sessionId, paths });
      toast.show(`Discarded changes to ${result.discarded} file${result.discarded === 1 ? "" : "s"}`, {
        tone: "neutral",
      });
    } catch (error) {
      toast.show(error instanceof Error ? error.message : "Could not discard.", { tone: "danger" });
    }
  };

  const stacked = width > 0 && width < STACK_BELOW;
  const indexBusy = stage.isPending || unstage.isPending || discard.isPending;
  const stagedCount = data?.staged.length ?? 0;
  const conflictCount = data?.conflicted?.length ?? 0;
  const clean =
    data != null &&
    data.staged.length === 0 &&
    data.unstaged.length === 0 &&
    data.untracked.length === 0 &&
    conflictCount === 0;
  const canPush = data?.has_remote !== false;

  // Outside a repository there is no branch to pick and no diff to show: rendering the picker
  // ("detached HEAD") and an empty "select a file" pane under the notice read as a broken repo.
  const problem = status.error ? classifyGitStatusError(status.error, host) : null;
  if (problem && problem.kind !== "error") {
    return (
      <View style={[styles.empty, { backgroundColor: tokens.bg0 }]}>
        <EmptyState icon={GitBranch} message={problem.message} />
      </View>
    );
  }

  let column: React.ReactNode;
  if (status.error) {
    column = (
      <View style={styles.errorBlock}>
        <Text style={[typeScale.sub, { color: tokens.danger }]}>{status.error.message}</Text>
        <Pressable
          onPress={() => status.refetch()}
          accessibilityRole="button"
          accessibilityLabel="Retry loading the working tree"
          style={[styles.retry, { borderColor: tokens.border, borderRadius: radii.radius4 }]}
        >
          <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>retry</Text>
        </Pressable>
      </View>
    );
  } else if (!data) {
    column = (
      <Text style={[typeScale.sub, styles.notice, { color: tokens.ink3 }]}>Loading working tree…</Text>
    );
  } else if (clean) {
    column = <EmptyState icon={GitBranch} message="Working tree clean — nothing to review." />;
  } else {
    column = (
      <GitFileList
        status={data}
        selected={selection}
        onSelect={selectFile}
        onStage={readOnly ? null : (paths) => stage.mutate({ session: sessionId, paths })}
        onUnstage={readOnly ? null : (paths) => unstage.mutate({ session: sessionId, paths })}
        onDiscard={readOnly ? null : (paths, kind) => setPendingDiscard({ paths, kind })}
        onResolveConflicts={
          readOnly
            ? null
            : (paths) => setResolving({ paths, operation: data.operation ?? null })
        }
        busy={indexBusy}
      />
    );
  }

  const discardCopy = pendingDiscard ? discardMessage(pendingDiscard.paths, pendingDiscard.kind) : null;

  const changedCount = entries.length + conflictCount;

  return (
    <View
      style={[styles.root, stacked && styles.rootStacked, { backgroundColor: tokens.bg0 }]}
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
    >
      {stacked ? (
        // Two full-height views instead of two cramped halves: the file list, sync and commit box
        // need room, and so does a diff, and a phone has neither to spare.
        <View style={[styles.viewSwitch, { borderBottomColor: tokens.border }]}>
          <Segmented<StackedView>
            options={[
              { value: "files", label: "Changes", badge: changedCount > 0 ? changedCount : undefined },
              { value: "diff", label: "Diff" },
            ]}
            value={stackedView}
            onChange={setStackedView}
            testID="git-dock-view"
          />
        </View>
      ) : null}
      <View
        style={[
          styles.column,
          stacked
            ? [styles.columnStacked, stackedView !== "files" && styles.hidden]
            : { width: FILE_COLUMN_WIDTH, borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth },
          { backgroundColor: tokens.bg1 },
        ]}
      >
        <GitBranchPicker
          sessionId={sessionId}
          branch={data?.branch ?? ""}
          baseBranch={data?.base_branch ?? null}
          readOnly={readOnly}
        />
        {data && !readOnly ? <GitSyncBar sessionId={sessionId} status={data} /> : null}
        <View style={styles.columnBody}>{column}</View>
        {data && !readOnly ? (
          <GitCommitBox
            branch={data.branch}
            baseBranch={data.base_branch}
            stagedCount={stagedCount}
            message={message}
            onChangeMessage={setMessage}
            onCommit={(action) => void runCommit(action)}
            committing={commit.isPending}
            pushing={push.isPending}
            canPush={canPush}
            action={commitAction}
            onChooseAction={setCommitAction}
            onGenerate={() => void generateMessage()}
            generating={generate.isPending}
            error={commit.error ?? push.error}
            result={commit.data ?? null}
            pushedTo={pushedTo}
          />
        ) : null}
      </View>

      <View style={[styles.diffHost, stacked && stackedView !== "diff" && styles.hidden]}>
        <GitDiffPane
          sessionId={sessionId}
          entries={entries}
          conflictCount={conflictCount}
          focusRequest={focusRequest}
          ignoreWhitespace={ignoreWhitespace}
          onToggleWhitespace={() => setIgnoreWhitespace((current) => !current)}
        />
      </View>

      <ConfirmDialog
        visible={pendingDiscard != null}
        title={discardCopy?.title ?? "Discard changes?"}
        message={discardCopy?.message}
        confirmLabel={pendingDiscard?.kind === "untracked" ? "Delete" : "Discard"}
        destructive
        loading={discard.isPending}
        onConfirm={() => void confirmDiscard()}
        onCancel={() => setPendingDiscard(null)}
      />
      <ConflictResolver
        sessionId={sessionId}
        request={resolving}
        onSent={() => {
          setResolving(null);
          toast.show("Asked Forge to resolve the conflicts", { tone: "success" });
        }}
        onFailed={(failure) => {
          setResolving(null);
          toast.show(failure, { tone: "danger" });
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  empty: { flex: 1, justifyContent: "center" },
  root: { flex: 1, flexDirection: "row", minHeight: 0 },
  rootStacked: { flexDirection: "column" },
  column: { flexShrink: 0 },
  columnStacked: { width: "100%", flex: 1, minHeight: 0 },
  diffHost: { flex: 1, minWidth: 0, minHeight: 0 },
  hidden: { display: "none" },
  viewSwitch: { paddingHorizontal: space.space12, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  columnBody: { flex: 1, minHeight: 0 },
  notice: { paddingHorizontal: space.space12, paddingVertical: space.space16 },
  errorBlock: { padding: space.space12, gap: space.space8 },
  retry: { alignSelf: "flex-start", borderWidth: 1, paddingHorizontal: space.space8, paddingVertical: 3 },
});
