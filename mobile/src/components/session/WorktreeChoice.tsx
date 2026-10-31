// The "Forge a task" worktree control: no worktree, a new one (optionally named), or launch
// into one that already exists. Replaces the bare on/off switch.
import { Check, GitBranch } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import type { WorktreeRow } from "../../lib/apiProjects";
import { branchNameProblem, type WorktreeMode } from "../../lib/newSessionDefaults";
import { useWorktrees } from "../../lib/queriesProjects";
import { repoRootOf } from "../../lib/worktreePaths";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { Segmented } from "../ds/Segmented";
import { Skeleton } from "../ds/Skeleton";
import { GitInitOffer } from "./GitInitOffer";

const MODE_OPTIONS: { value: WorktreeMode; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "new", label: "New worktree" },
  { value: "existing", label: "Existing" },
];

export interface WorktreeChoiceProps {
  cwd: string;
  mode: WorktreeMode;
  onModeChange: (mode: WorktreeMode) => void;
  branch: string;
  onBranchChange: (branch: string) => void;
  branchHint: string;
  existingPath: string | null;
  onExistingChange: (path: string | null) => void;
  /** false = known not to be a git repo; null = unknown (manual path). */
  isGitRepo: boolean | null;
  /** The Home scope is never a repository and is not offered `git init`. */
  isHome: boolean;
}

export function WorktreeChoice(props: WorktreeChoiceProps) {
  const { cwd, mode, isGitRepo, isHome } = props;
  const tokens = useTokens();

  if (isGitRepo === false) {
    return (
      <View style={styles.wrap}>
        <Text style={[typeScale.body, { color: tokens.ink }]}>Git worktree</Text>
        {isHome ? (
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Home is a scratch folder, so there is nothing to isolate.</Text>
        ) : (
          <GitInitOffer path={cwd} />
        )}
      </View>
    );
  }

  const problem = mode === "new" ? branchNameProblem(props.branch) : null;
  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.body, { color: tokens.ink }]}>Git worktree</Text>
      <Segmented options={MODE_OPTIONS} value={mode} onChange={props.onModeChange} testID="worktree-mode" />
      {mode === "off" ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Works directly in the project folder.</Text>
      ) : mode === "new" ? (
        <>
          <Input
            label="Branch name (optional)"
            value={props.branch}
            onChangeText={props.onBranchChange}
            placeholder={props.branchHint}
            mono
            autoCapitalize="none"
            autoCorrect={false}
            error={problem ?? undefined}
            accessibilityLabel="New worktree branch name"
          />
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            A separate checkout on its own branch. Leave the name empty for a friendly random one.
          </Text>
        </>
      ) : (
        <ExistingWorktrees cwd={cwd} selected={props.existingPath} onSelect={props.onExistingChange} />
      )}
    </View>
  );
}

function ExistingWorktrees({
  cwd,
  selected,
  onSelect,
}: {
  cwd: string;
  selected: string | null;
  onSelect: (path: string | null) => void;
}) {
  const tokens = useTokens();
  const query = useWorktrees(repoRootOf(cwd));
  const rows: WorktreeRow[] = (query.data?.worktrees ?? []).filter((row) => !row.main);

  if (query.isLoading) return <Skeleton height={56} width="100%" />;
  if (query.isError) {
    return <Text style={[typeScale.sub, { color: tokens.danger }]}>Could not list worktrees for this project.</Text>;
  }
  if (rows.length === 0) {
    return <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No other worktrees yet. Choose New worktree to create one.</Text>;
  }
  return (
    <View accessibilityRole="radiogroup">
      {rows.map((row, index) => {
        const active = row.path === selected;
        return (
          <ListRow
            key={row.path}
            title={row.branch ?? `detached ${row.head.slice(0, 7)}`}
            subtitle={row.path}
            leading={<GitBranch size={16} strokeWidth={1.75} color={active ? tokens.accent : tokens.ink3} />}
            trailing={
              <View style={styles.trailing}>
                {row.dirty ? <Badge label="uncommitted" tone="warn" lowercase /> : null}
                {active ? <Check size={16} strokeWidth={2} color={tokens.success} /> : null}
              </View>
            }
            onPress={() => onSelect(active ? null : row.path)}
            accessibilityRole="radio"
            accessibilityLabel={`${row.branch ?? "detached"}${row.dirty ? ", has uncommitted changes" : ""}${active ? ", selected" : ""}`}
            showSeparator={index < rows.length - 1}
          />
        );
      })}
      {selected ? null : <Text style={[typeScale.sub, styles.pickHint, { color: tokens.ink3 }]}>Pick a worktree to continue.</Text>}
    </View>
  );
}

const styles = StyleSheet.create({
  trailing: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  pickHint: { paddingTop: space.space8 },
  wrap: { gap: space.space8, paddingVertical: space.space12 },
});
