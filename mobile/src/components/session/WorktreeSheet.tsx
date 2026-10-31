// Session-level worktree actions: move this thread into a new worktree (choosing what happens
// to uncommitted changes) and pull the latest upstream with a conflict handoff to the agent.
import { GitBranch, GitPullRequestArrow } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError } from "../../lib/api";
import { branchNameProblem } from "../../lib/newSessionDefaults";
import { usePullWorktree } from "../../lib/queriesProjects";
import { pullResultView, WORKTREE_PULL_COMMAND, worktreeNewCommand } from "../../lib/worktreeCommands";
import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Banner } from "../ds/Banner";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { SectionHeader } from "../ds/SectionHeader";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

type DirtyChoice = "carry" | "leave";

const DIRTY_OPTIONS: { value: DirtyChoice; label: string }[] = [
  { value: "carry", label: "Carry my changes" },
  { value: "leave", label: "Leave them here" },
];

export interface WorktreeSheetProps {
  visible: boolean;
  onClose: () => void;
  cwd: string;
  send: (input: RemoteInput) => boolean;
}

export function WorktreeSheet({ visible, onClose, cwd, send }: WorktreeSheetProps) {
  const tokens = useTokens();
  const toast = useToast();
  const pull = usePullWorktree();
  const [name, setName] = useState("");
  const [dirty, setDirty] = useState<DirtyChoice>("carry");
  const problem = branchNameProblem(name);

  const dispatch = (text: string, sent: string) => {
    if (send({ kind: "prompt", text })) {
      toast.show(sent);
      return true;
    }
    toast.show("not sent, reconnect and try again", { tone: "danger" });
    return false;
  };

  const move = () => {
    if (problem) return;
    if (dispatch(worktreeNewCommand(name, dirty === "carry"), "moving to a new worktree")) onClose();
  };

  const view = pull.data ? pullResultView(pull.data) : null;
  const pullError = pull.error instanceof ApiError ? pull.error.message : pull.isError ? "Could not pull" : null;

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Worktree" snapPoints={[0.85]}>
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Worktree</Text>

        <View>
          <SectionHeader>Move this thread</SectionHeader>
          <Text style={[typeScale.sub, styles.copy, { color: tokens.ink3 }]}>
            Continue in a fresh worktree on its own branch. Uncommitted changes can come along or stay behind.
          </Text>
        </View>
        <Input
          label="Branch name (optional)"
          value={name}
          onChangeText={setName}
          placeholder="calm-otter"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          error={problem ?? undefined}
          accessibilityLabel="New worktree branch name"
        />
        <Segmented options={DIRTY_OPTIONS} value={dirty} onChange={setDirty} testID="worktree-dirty" />
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          {dirty === "carry"
            ? "Your uncommitted changes move into the new worktree."
            : "Your uncommitted changes stay in the current checkout."}
        </Text>
        <Button
          label="Move to new worktree"
          icon={<GitBranch size={15} strokeWidth={1.8} color={tokens.onAccent} />}
          onPress={move}
          disabled={problem !== null}
          fullWidth
        />

        <View>
          <SectionHeader>Pull latest</SectionHeader>
          <Text style={[typeScale.sub, styles.copy, { color: tokens.ink3 }]}>
            Fetches and merges upstream. Uncommitted work is stashed first and put back afterwards.
          </Text>
        </View>
        <Button
          label="Pull latest"
          variant="secondary"
          icon={<GitPullRequestArrow size={15} strokeWidth={1.8} color={tokens.ink2} />}
          loading={pull.isPending}
          onPress={() => pull.mutate(cwd)}
          disabled={!cwd}
          fullWidth
        />
        {view && pull.data ? (
          <View style={styles.result} accessibilityLiveRegion="polite">
            <Banner
              tone={view.tone === "danger" ? "danger" : view.tone === "warn" ? "warn" : "neutral"}
              message={view.title}
            />
            {pull.data.message ? <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{pull.data.message}</Text> : null}
            {pull.data.conflicts.map((file) => (
              <Text key={file} style={[typeScale.codeSmall, { color: tokens.warn }]} numberOfLines={1}>
                {file}
              </Text>
            ))}
            {view.canHandoff && pull.data.handoff_prompt ? (
              <Button
                label="Hand to the agent"
                onPress={() => {
                  if (dispatch(pull.data?.handoff_prompt ?? WORKTREE_PULL_COMMAND, "handed to the agent")) onClose();
                }}
                fullWidth
              />
            ) : null}
          </View>
        ) : null}
        {pullError ? (
          <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
            {pullError}
          </Text>
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  copy: { paddingHorizontal: space.space16 },
  result: { gap: space.space8 },
});
