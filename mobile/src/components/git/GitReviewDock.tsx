// Machined git review dock (docs/design/machined "Forge Machined - Desktop.dc.html" L265-308,
// D Git Review; INVENTORY.md L76-77 — the staged/unstaged list + split-diff browser + commit
// box had no equivalent in the app before this).
//
// Desktop chrome, but it does not assume a 1440px window: the dock measures itself and stacks
// the file column above the diff pane when its container is narrow, and the pane drops to a
// unified diff when two columns would be narrower than a code line.
//
// Every row, count, branch name and error string here comes from `/api/git/*` — the dock has
// no local model of the working tree, and a failed request surfaces the daemon's own text
// rather than a generic "something went wrong".
import { GitBranch } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { GitCommitBox } from "./GitCommitBox";
import { GitDiffPane } from "./GitDiffPane";
import { GitFileList, type GitSelection } from "./GitFileList";
import { type GitStatusResponse } from "../../lib/api";
import { useCommitStaged, useGitDiff, useGitStatus, useStagePaths, useUnstagePaths } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";

const FILE_COLUMN_WIDTH = 264;
/** Below this the 264px column plus a readable diff no longer fit side by side. */
const STACK_BELOW = 700;
const STACKED_COLUMN_HEIGHT = 260;

/** The daemon's repo-resolution failures ([`repo_root_of`] in serve_git.rs) are facts about the
 * session, not transient faults — retrying them is pointless, so they get the quiet empty-state
 * treatment instead of an error with a Retry button. */
function isNotARepo(message: string): boolean {
  return message.includes("not inside a git repository") || message.includes("no working directory");
}

function firstSelection(status: GitStatusResponse): GitSelection | null {
  const staged = status.staged[0];
  if (staged) return { path: staged.path, staged: true };
  const unstaged = status.unstaged[0] ?? status.untracked[0];
  if (unstaged) return { path: unstaged.path, staged: false };
  return null;
}

export function GitReviewDock({ sessionId }: { sessionId: string }): React.JSX.Element {
  const tokens = useTokens();
  const [width, setWidth] = useState(0);
  const [selection, setSelection] = useState<GitSelection | null>(null);
  const [message, setMessage] = useState("");

  const status = useGitStatus(sessionId);
  const diff = useGitDiff(sessionId, selection?.path ?? null, selection?.staged ?? false);
  const stage = useStagePaths();
  const unstage = useUnstagePaths();
  const commit = useCommitStaged();

  const data = status.data;

  // Staging moves a path between buckets, which would otherwise strand the selection on a
  // bucket that no longer holds it: follow the file across instead of clearing the pane.
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

  const stacked = width > 0 && width < STACK_BELOW;
  const indexBusy = stage.isPending || unstage.isPending;
  const stagedCount = data?.staged.length ?? 0;
  const clean =
    data != null && data.staged.length === 0 && data.unstaged.length === 0 && data.untracked.length === 0;

  let column: React.ReactNode;
  if (status.error) {
    const text = status.error.message;
    column = isNotARepo(text) ? (
      <EmptyState icon={GitBranch} message={text} />
    ) : (
      <View style={styles.errorBlock}>
        <Text style={[typeScale.sub, { color: tokens.danger }]}>{text}</Text>
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
        onSelect={setSelection}
        onStage={(paths) => stage.mutate({ session: sessionId, paths })}
        onUnstage={(paths) => unstage.mutate({ session: sessionId, paths })}
        busy={indexBusy}
      />
    );
  }

  return (
    <View
      style={[styles.root, stacked && styles.rootStacked, { backgroundColor: tokens.bg0 }]}
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
    >
      <View
        style={[
          styles.column,
          stacked
            ? { width: "100%", height: STACKED_COLUMN_HEIGHT, borderBottomColor: tokens.border, borderBottomWidth: StyleSheet.hairlineWidth }
            : { width: FILE_COLUMN_WIDTH, borderRightColor: tokens.border, borderRightWidth: StyleSheet.hairlineWidth },
          { backgroundColor: tokens.bg1 },
        ]}
      >
        <View style={styles.columnBody}>{column}</View>
        {data ? (
          <GitCommitBox
            branch={data.branch}
            baseBranch={data.base_branch}
            stagedCount={stagedCount}
            message={message}
            onChangeMessage={setMessage}
            onCommit={() =>
              commit.mutate(
                { session: sessionId, message },
                { onSuccess: () => setMessage("") },
              )
            }
            committing={commit.isPending}
            error={commit.error}
            result={commit.data ?? null}
          />
        ) : null}
      </View>

      <GitDiffPane
        file={diff.data?.files[0] ?? null}
        staged={selection?.staged ?? false}
        hasSelection={selection != null}
        loading={diff.isLoading}
        error={diff.error}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, flexDirection: "row", minHeight: 0 },
  rootStacked: { flexDirection: "column" },
  column: { flexShrink: 0 },
  columnBody: { flex: 1, minHeight: 0 },
  notice: { paddingHorizontal: space.space12, paddingVertical: space.space16 },
  errorBlock: { padding: space.space12, gap: space.space8 },
  retry: { alignSelf: "flex-start", borderWidth: 1, paddingHorizontal: space.space8, paddingVertical: 3 },
});
