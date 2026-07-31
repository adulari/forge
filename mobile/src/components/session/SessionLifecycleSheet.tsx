import {
  Archive,
  ExternalLink,
  Pencil,
  Play,
  Trash2,
} from "lucide-react-native";
import { router, usePathname } from "expo-router";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError } from "../../lib/api";
import {
  useArchiveSession,
  useCreateSession,
  useDeleteSession,
  useRenameSession,
} from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

export interface SessionLifecycleTarget {
  id: string;
  title: string;
  cwd: string;
  archived: boolean;
  running: boolean;
}

export interface SessionLifecycleSheetProps {
  target: SessionLifecycleTarget | null;
  visible: boolean;
  onClose: () => void;
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof ApiError ? error.message : fallback;
}

export function SessionLifecycleSheet({
  target,
  visible,
  onClose,
}: SessionLifecycleSheetProps) {
  const tokens = useTokens();
  const toast = useToast();
  const pathname = usePathname();
  const createSession = useCreateSession();
  const archiveSession = useArchiveSession();
  const renameSession = useRenameSession();
  const deleteSession = useDeleteSession();
  const [current, setCurrent] = useState<SessionLifecycleTarget | null>(target);
  const [mode, setMode] = useState<"actions" | "rename">("actions");
  const [title, setTitle] = useState(target?.title ?? "");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const targetId = target?.id;
  const targetTitle = target?.title ?? "";
  const targetCwd = target?.cwd ?? "";
  const targetArchived = target?.archived ?? false;
  const targetRunning = target?.running ?? false;

  useEffect(() => {
    if (!targetId) return;
    setCurrent({
      id: targetId,
      title: targetTitle,
      cwd: targetCwd,
      archived: targetArchived,
      running: targetRunning,
    });
    setTitle(targetTitle);
    setMode("actions");
  }, [targetArchived, targetCwd, targetId, targetRunning, targetTitle]);

  const close = () => {
    setMode("actions");
    onClose();
  };

  const open = () => {
    if (!current) return;
    if (current.running) {
      close();
      router.push(`/session/${current.id}`);
      return;
    }
    createSession.mutate(
      { resume: current.id },
      {
        onSuccess: (session) => {
          close();
          router.push(`/session/${session.id}`);
        },
        onError: (error) =>
          toast.show(errorMessage(error, "could not resume session."), { tone: "danger" }),
      },
    );
  };

  const archive = () => {
    if (!current) return;
    archiveSession.mutate(current.id, {
      onSuccess: () => {
        toast.show("session archived");
        close();
        if (pathname.startsWith(`/session/${current.id}`)) router.replace("/");
      },
      onError: (error) =>
        toast.show(errorMessage(error, "could not archive session."), { tone: "danger" }),
    });
  };

  const rename = () => {
    if (!current) return;
    const nextTitle = title.trim();
    if (!nextTitle) return;
    renameSession.mutate(
      { id: current.id, title: nextTitle },
      {
        onSuccess: () => {
          setCurrent({ ...current, title: nextTitle });
          toast.show("session renamed");
          close();
        },
        onError: (error) =>
          toast.show(errorMessage(error, "could not rename session."), { tone: "danger" }),
      },
    );
  };

  const remove = () => {
    if (!current) return;
    deleteSession.mutate(current.id, {
      onSuccess: () => {
        setConfirmDelete(false);
        toast.show("session deleted");
        if (pathname.startsWith(`/session/${current.id}`)) router.replace("/");
      },
      onError: (error) => {
        setConfirmDelete(false);
        toast.show(errorMessage(error, "could not delete session."), { tone: "danger" });
      },
    });
  };

  return (
    <>
      <Sheet
        visible={visible && current != null}
        onClose={close}
        accessibilityLabel="Session lifecycle actions"
        snapPoints={[0.7]}
      >
        {current ? (
          <View style={styles.content}>
            <Text style={[typeScale.heading, { color: tokens.ink }]} numberOfLines={1}>
              {current.title || `Session ${current.id.slice(0, 8)}`}
            </Text>
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
              {current.cwd}
            </Text>
            {mode === "rename" ? (
              <View style={styles.rename}>
                <Input
                  label="Session title"
                  value={title}
                  onChangeText={setTitle}
                  maxLength={120}
                  autoFocus
                  returnKeyType="done"
                  onSubmitEditing={rename}
                />
                <View style={styles.buttons}>
                  <Button label="Cancel" variant="secondary" onPress={() => setMode("actions")} />
                  <Button
                    label="Rename"
                    variant="primary"
                    onPress={rename}
                    disabled={!title.trim() || renameSession.isPending}
                    loading={renameSession.isPending}
                  />
                </View>
              </View>
            ) : (
              <View>
                <ListRow
                  title={current.running ? "Open session" : "Resume session"}
                  subtitle={
                    current.archived && !current.running
                      ? "Restores this archived session and its full history"
                      : undefined
                  }
                  leading={
                    current.running
                      ? <ExternalLink size={19} color={tokens.ink2} />
                      : <Play size={19} color={tokens.ink2} />
                  }
                  onPress={open}
                />
                <ListRow
                  title="Rename session"
                  leading={<Pencil size={19} color={tokens.ink2} />}
                  onPress={() => setMode("rename")}
                />
                {!current.archived ? (
                  <ListRow
                    title="Archive session"
                    subtitle={current.running ? "Stops active work and keeps full history" : undefined}
                    leading={<Archive size={19} color={tokens.ink2} />}
                    onPress={archive}
                    disabled={archiveSession.isPending}
                  />
                ) : null}
                {!current.running ? (
                  <ListRow
                    title="Delete permanently…"
                    subtitle="Removes the session and transcript; managed worktrees must be resolved first"
                    leading={<Trash2 size={19} color={tokens.danger} />}
                    onPress={() => {
                      close();
                      setConfirmDelete(true);
                    }}
                    showSeparator={false}
                  />
                ) : null}
              </View>
            )}
          </View>
        ) : null}
      </Sheet>
      <ConfirmDialog
        visible={confirmDelete && current != null}
        title="Delete this session permanently?"
        message={
          current
            ? `“${current.title || current.id.slice(0, 8)}” and its transcript will be deleted. This cannot be undone.`
            : ""
        }
        confirmLabel="Delete"
        destructive
        loading={deleteSession.isPending}
        onConfirm={remove}
        onCancel={() => setConfirmDelete(false)}
      />
    </>
  );
}

const styles = StyleSheet.create({
  content: {
    paddingHorizontal: space.space16,
    paddingBottom: space.space32,
    gap: space.space4,
  },
  rename: { gap: space.space16, paddingTop: space.space16 },
  buttons: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8 },
});
