import { Check, ChevronDown, GitBranch, Plus, Search } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import {
  canSelectGitBranch,
  filterGitBranches,
  gitBranchSubtitle,
} from "./gitBranchModel";
import { type GitBranchRow } from "../../lib/api";
import {
  useCreateGitBranch,
  useGitBranches,
  useSwitchGitBranch,
} from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";

export interface GitBranchPickerProps {
  sessionId: string;
  branch: string;
  baseBranch: string | null;
  /** Over Anywhere the host refuses branch switches, so the picker renders as a label. */
  readOnly?: boolean;
}

export function GitBranchPicker({
  sessionId,
  branch,
  baseBranch,
  readOnly = false,
}: GitBranchPickerProps): React.JSX.Element {
  const tokens = useTokens();
  const branches = useGitBranches(sessionId);
  const switchBranch = useSwitchGitBranch();
  const createBranch = useCreateGitBranch();
  const [visible, setVisible] = useState(false);
  const [query, setQuery] = useState("");
  const [newBranch, setNewBranch] = useState("");

  const data = branches.data;
  const activeBranch = data?.current ?? branch;
  const blockedReason = data?.actions_blocked_reason ?? null;
  const actionBusy = switchBranch.isPending || createBranch.isPending;
  const actionError = switchBranch.error ?? createBranch.error;
  const rows = useMemo(
    () => filterGitBranches(data?.branches ?? [], query),
    [data?.branches, query],
  );

  const close = () => {
    setVisible(false);
    setQuery("");
    switchBranch.reset();
    createBranch.reset();
  };

  const select = (row: GitBranchRow) => {
    if (!canSelectGitBranch(row, blockedReason, actionBusy)) return;
    switchBranch.mutate(
      { session: sessionId, branch: row.name },
      { onSuccess: close },
    );
  };

  const create = () => {
    const name = newBranch.trim();
    if (!name || blockedReason || actionBusy) return;
    createBranch.mutate(
      { session: sessionId, name },
      {
        onSuccess: () => {
          setNewBranch("");
          close();
        },
      },
    );
  };

  return (
    <>
      <Pressable
        onPress={readOnly ? undefined : () => setVisible(true)}
        disabled={readOnly}
        accessibilityRole={readOnly ? "text" : "button"}
        accessibilityLabel={
          readOnly
            ? `Current branch ${activeBranch || "detached HEAD"} — switching needs a direct connection`
            : `Open branches and worktrees, current branch ${activeBranch || "detached HEAD"}`
        }
        style={({ pressed }) => [
          styles.trigger,
          {
            backgroundColor: pressed ? tokens.bg3 : tokens.bg1,
            borderBottomColor: tokens.border,
          },
        ]}
      >
        <GitBranch size={14} strokeWidth={1.8} color={tokens.ink3} />
        <Text
          style={[typeScale.monoMeta, styles.triggerLabel, { color: tokens.ink2 }]}
          numberOfLines={1}
        >
          {activeBranch || "detached HEAD"}
        </Text>
        {baseBranch && baseBranch !== activeBranch ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            → {baseBranch}
          </Text>
        ) : null}
        <ChevronDown size={13} strokeWidth={1.8} color={tokens.ink4} />
      </Pressable>

      <Sheet
        visible={visible}
        onClose={close}
        accessibilityLabel="Branches and worktrees"
        snapPoints={[1]}
        maxHeightRatio={0.86}
      >
        <View style={styles.sheet}>
          <View style={styles.heading}>
            <View style={styles.headingCopy}>
              <Text style={[typeScale.heading, { color: tokens.ink }]}>Branches & worktrees</Text>
              <Text style={[typeScale.sub, { color: tokens.ink3 }]} numberOfLines={1}>
                {data?.root ?? "Loading repository…"}
              </Text>
            </View>
            <GitBranch size={20} strokeWidth={1.7} color={tokens.accent} />
          </View>

          {blockedReason ? (
            <View
              style={[
                styles.notice,
                {
                  backgroundColor: tokens.bg2,
                  borderColor: tokens.border,
                  borderRadius: radii.radius8,
                },
              ]}
            >
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{blockedReason}</Text>
            </View>
          ) : (
            <View style={styles.createRow}>
              <Input
                label="New branch"
                value={newBranch}
                onChangeText={setNewBranch}
                onSubmitEditing={create}
                placeholder="feature/my-change"
                autoCapitalize="none"
                autoCorrect={false}
                mono
                clearable
                containerStyle={styles.createInput}
                accessibilityLabel="New branch name"
              />
              <Button
                label="Create & switch"
                onPress={create}
                disabled={newBranch.trim().length === 0}
                loading={createBranch.isPending}
                variant="secondary"
                icon={<Plus size={15} strokeWidth={1.8} color={tokens.ink2} />}
                style={styles.createButton}
              />
            </View>
          )}

          <Input
            value={query}
            onChangeText={setQuery}
            placeholder="Search branches"
            autoCapitalize="none"
            autoCorrect={false}
            clearable
            leading={<Search size={16} strokeWidth={1.7} color={tokens.ink3} />}
            accessibilityLabel="Search branches"
          />

          {actionError ? (
            <Text style={[typeScale.sub, { color: tokens.danger }]}>{actionError.message}</Text>
          ) : null}

          <ScrollView
            style={styles.list}
            contentContainerStyle={styles.listContent}
            keyboardShouldPersistTaps="handled"
          >
            {branches.error ? (
              <View style={styles.queryError}>
                <Text style={[typeScale.sub, { color: tokens.danger }]}>
                  {branches.error.message}
                </Text>
                <Button
                  label="Retry"
                  onPress={() => void branches.refetch()}
                  variant="ghost"
                  loading={branches.isFetching}
                />
              </View>
            ) : branches.isLoading ? (
              <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>
                Loading branches…
              </Text>
            ) : rows.length === 0 ? (
              <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>
                No matching branches.
              </Text>
            ) : (
              rows.map((row, index) => {
                const occupiedElsewhere = row.worktree != null && !row.current;
                const selectable = canSelectGitBranch(row, blockedReason, actionBusy);
                return (
                  <ListRow
                    key={`${row.remote ? "remote" : "local"}:${row.name}`}
                    title={row.name}
                    subtitle={gitBranchSubtitle(row)}
                    leading={
                      row.current ? (
                        <Check size={16} strokeWidth={2} color={tokens.success} />
                      ) : (
                        <GitBranch size={16} strokeWidth={1.7} color={tokens.ink3} />
                      )
                    }
                    trailing={
                      <Text
                        style={[
                          typeScale.monoMeta,
                          { color: row.remote ? tokens.ink4 : tokens.ink3 },
                        ]}
                      >
                        {row.oid}
                      </Text>
                    }
                    onPress={selectable ? () => select(row) : undefined}
                    disabled={occupiedElsewhere}
                    showSeparator={index < rows.length - 1}
                    accessibilityLabel={`${row.name}, ${gitBranchSubtitle(row)}`}
                  />
                );
              })
            )}
            {(data?.truncated ?? 0) > 0 ? (
              <Text style={[typeScale.monoMeta, styles.empty, { color: tokens.ink4 }]}>
                {data?.truncated} more refs omitted
              </Text>
            ) : null}
          </ScrollView>
        </View>
      </Sheet>
    </>
  );
}

const styles = StyleSheet.create({
  trigger: {
    minHeight: 35,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: space.space8,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  triggerLabel: {
    flexShrink: 1,
    fontFamily: monoFamily.regular,
  },
  sheet: {
    minHeight: 420,
    paddingHorizontal: space.space16,
    paddingBottom: space.space16,
    gap: space.space12,
  },
  heading: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
  },
  headingCopy: { flex: 1, gap: 2 },
  notice: {
    borderWidth: 1,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
  },
  createRow: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: space.space8,
  },
  createInput: { flex: 1 },
  createButton: { minHeight: 44 },
  list: { maxHeight: 420 },
  listContent: { paddingBottom: space.space8 },
  queryError: {
    alignItems: "flex-start",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
  },
  empty: {
    paddingHorizontal: space.space16,
    paddingVertical: space.space16,
    textAlign: "center",
  },
});
