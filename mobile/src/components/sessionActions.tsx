// Shared session action sheet (Archive / Merge / Discard) — mounted by Fleet and reused by
// History (BUILD_PLAN §7 W2/W3). Only needs {id, title, worktree}, so it works for both
// SessionRow and PastSessionRow without a screen-specific shape.
import React, { useCallback, useEffect, useState } from "react";
import { Modal, Pressable, Text, View } from "react-native";

import { ApiError, type ErrorBody, type MergeDirtyConflictResponse } from "../lib/api";
import {
  useArchiveSession,
  useDiscardSession,
  useMergeSession,
} from "../lib/queries";
import { Card, ConfirmButton } from "./ui";

export interface SessionActionTarget {
  id: string;
  title: string;
  worktree: string | null;
}

type Stage =
  | "menu"
  | "archive-confirm"
  | "merge-result"
  | "discard-confirm-1"
  | "discard-confirm-2"
  | "discard-result";

export function useSessionActions() {
  const [target, setTarget] = useState<SessionActionTarget | null>(null);
  const open = useCallback((t: SessionActionTarget) => setTarget(t), []);
  const close = useCallback(() => setTarget(null), []);
  return { open, sheet: <SessionActionSheet target={target} onClose={close} /> };
}

interface ActionRowProps {
  label: string;
  hint: string;
  tone?: "ink" | "no";
  onPress: () => void;
}

function ActionRow({ label, hint, tone = "ink", onPress }: ActionRowProps) {
  return (
    <Card onPress={onPress} className="gap-2">
      <Text
        className={tone === "no" ? "text-no text-[15px] font-semibold" : "text-ink text-[15px] font-semibold"}
      >
        {label}
      </Text>
      <Text className="text-dim text-[12px]">{hint}</Text>
    </Card>
  );
}

function SessionActionSheet({
  target,
  onClose,
}: {
  target: SessionActionTarget | null;
  onClose: () => void;
}) {
  const [visible, setVisible] = useState(false);
  const [displayTarget, setDisplayTarget] = useState<SessionActionTarget | null>(null);
  const [stage, setStage] = useState<Stage>("menu");

  const archive = useArchiveSession();
  const merge = useMergeSession();
  const discard = useDiscardSession();

  useEffect(() => {
    if (target) {
      setDisplayTarget(target);
      setVisible(true);
      setStage("menu");
      archive.reset();
      merge.reset();
      discard.reset();
    } else {
      setVisible(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target]);

  if (!displayTarget) return null;

  const handleClose = () => onClose();

  const handleArchive = () => {
    archive.mutate(displayTarget.id, { onSuccess: handleClose });
  };

  const handleMerge = () => {
    // Merge never confirms up-front (mirrors app.js) — conflicts/dirty state are reported
    // inline on the result stage instead of gating behind a prompt.
    setStage("merge-result");
    merge.mutate(displayTarget.id);
  };

  const handleDiscard = () => {
    discard.mutate(displayTarget.id, { onSuccess: () => setStage("discard-result") });
  };

  const mergeErrorBody =
    merge.error instanceof ApiError
      ? (merge.error.body as MergeDirtyConflictResponse | undefined)
      : undefined;
  const discardErrorBody =
    discard.error instanceof ApiError ? (discard.error.body as ErrorBody | undefined) : undefined;

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={handleClose}>
      <Pressable className="flex-1 bg-black/60 justify-end" onPress={handleClose}>
        <Pressable onPress={() => undefined}>
          <Card variant="feature" className="mx-12 mb-16 gap-10">
            <View className="flex-row items-center gap-8">
              <Text numberOfLines={1} className="flex-1 text-ink text-[16px] font-bold">
                {displayTarget.title || `#${displayTarget.id.slice(0, 8)}`}
              </Text>
              <Pressable
                onPress={handleClose}
                hitSlop={12}
                accessibilityRole="button"
                accessibilityLabel="Close"
                style={{ minWidth: 44, minHeight: 44, alignItems: "flex-end", justifyContent: "center" }}
              >
                <Text className="text-dim text-[18px]">×</Text>
              </Pressable>
            </View>

            {stage === "menu" ? (
              <View className="gap-6">
                <ActionRow
                  label="Archive"
                  hint="Stops the session; history is kept."
                  onPress={() => setStage("archive-confirm")}
                />
                {displayTarget.worktree ? (
                  <>
                    <ActionRow
                      label={merge.isPending ? "Merging…" : "Merge"}
                      hint="Merge the worktree into its base branch."
                      onPress={handleMerge}
                    />
                    <ActionRow
                      label="Discard"
                      hint="Delete the branch and worktree — destructive."
                      tone="no"
                      onPress={() => setStage("discard-confirm-1")}
                    />
                  </>
                ) : null}
              </View>
            ) : null}

            {stage === "archive-confirm" ? (
              <View className="gap-10">
                <Text className="text-dim text-[13px]">
                  Archive this session? It stops running — history is kept.
                </Text>
                {archive.isError ? (
                  <Text className="text-no text-[12px]">{(archive.error as Error).message}</Text>
                ) : null}
                <View className="flex-row gap-8">
                  <ConfirmButton
                    label={archive.isPending ? "Archiving…" : "Archive"}
                    tone="no"
                    onPress={handleArchive}
                    loading={archive.isPending}
                    className="flex-1"
                  />
                </View>
                <ActionRow label="Back" hint="Return to actions." onPress={() => setStage("menu")} />
              </View>
            ) : null}

            {stage === "merge-result" ? (
              <View className="gap-10">
                {merge.isPending ? (
                  <Text className="text-dim text-[13px]">Merging worktree into its base branch…</Text>
                ) : merge.isSuccess && merge.data ? (
                  <Text className="text-ok text-[13px]">
                    Merged into {merge.data.branch}. Worktree removed.
                  </Text>
                ) : mergeErrorBody?.conflicts?.length ? (
                  <View className="gap-6">
                    <Text className="text-no text-[13px] font-semibold">
                      Merge conflicts on {mergeErrorBody.branch ?? "base branch"}:
                    </Text>
                    {mergeErrorBody.conflicts.map((f) => (
                      <Text key={f} numberOfLines={1} className="text-dim text-[12px]">
                        {f}
                      </Text>
                    ))}
                    <Text className="text-dim text-[12px]">
                      Session stopped; worktree kept for manual resolution.
                    </Text>
                  </View>
                ) : mergeErrorBody?.dirty_files?.length ? (
                  <View className="gap-6">
                    <Text className="text-no text-[13px] font-semibold">
                      Base repo has uncommitted changes:
                    </Text>
                    {mergeErrorBody.dirty_files.map((f) => (
                      <Text key={f} numberOfLines={1} className="text-dim text-[12px]">
                        {f}
                      </Text>
                    ))}
                    <Text className="text-dim text-[12px]">
                      Session left running — commit or stash, then retry.
                    </Text>
                  </View>
                ) : merge.isError ? (
                  <Text className="text-no text-[13px]">{(merge.error as Error).message}</Text>
                ) : null}
                {!merge.isPending ? (
                  <ActionRow label="Close" hint="Dismiss." onPress={handleClose} />
                ) : null}
              </View>
            ) : null}

            {stage === "discard-confirm-1" ? (
              <View className="gap-10">
                <Text className="text-no text-[13px] font-semibold">
                  Discard this worktree AND its branch?
                </Text>
                <Text numberOfLines={1} className="text-dim text-[13px]">
                  {displayTarget.worktree}
                </Text>
                <Text className="text-dim text-[12px]">
                  Unmerged work is lost. This cannot be undone.
                </Text>
                <View className="flex-row gap-8">
                  <ConfirmButton
                    label="Continue"
                    tone="no"
                    onPress={() => setStage("discard-confirm-2")}
                    className="flex-1"
                  />
                </View>
                <ActionRow label="Back" hint="Return to actions." onPress={() => setStage("menu")} />
              </View>
            ) : null}

            {stage === "discard-confirm-2" ? (
              <View className="gap-10">
                <Text className="text-no text-[13px] font-semibold">
                  Are you sure? This permanently deletes the branch and worktree.
                </Text>
                {discardErrorBody?.error ? (
                  <Text className="text-no text-[12px]">{discardErrorBody.error}</Text>
                ) : null}
                <View className="flex-row gap-8">
                  <ConfirmButton
                    label={discard.isPending ? "Discarding…" : "Yes, discard everything"}
                    tone="no"
                    onPress={handleDiscard}
                    loading={discard.isPending}
                    className="flex-1"
                  />
                </View>
                <ActionRow label="Cancel" hint="Return to actions." onPress={() => setStage("menu")} />
              </View>
            ) : null}

            {stage === "discard-result" && discard.data ? (
              <View className="gap-8">
                <Text className="text-ok text-[13px]">
                  Discarded branch {discard.data.branch}.
                </Text>
                {discard.data.warnings.length ? (
                  <View className="gap-4">
                    {discard.data.warnings.map((w) => (
                      <Text key={w} className="text-dim text-[12px]">
                        {w}
                      </Text>
                    ))}
                  </View>
                ) : null}
                <ActionRow label="Done" hint="Dismiss." onPress={handleClose} />
              </View>
            ) : null}
          </Card>
        </Pressable>
      </Pressable>
    </Modal>
  );
}
