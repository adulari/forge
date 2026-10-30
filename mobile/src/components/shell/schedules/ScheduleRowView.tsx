import { Pause, Pencil, Play, Trash2 } from "lucide-react-native";
import { useRouter } from "expo-router";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { ScheduleRow, ScheduleRunStatus } from "../../../lib/api";
import {
  usePauseSchedule,
  useResumeSchedule,
  useDeleteSchedule,
  useRunScheduleNow,
} from "../../../lib/queries";
import { useTokens } from "../../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../../theme/tokens";
import { formatCwd, tabularNums, type as typeScale } from "../../../theme/typography";
import { ConfirmDialog } from "../../ds/ConfirmDialog";
import { useClockSeconds } from "../../ds/relativeClock";
import { cadenceLabel, lastResultLine, nextRunLabel } from "./scheduleModel";

function statusColor(
  tokens: ReturnType<typeof useTokens>,
  enabled: boolean,
  status: ScheduleRunStatus | undefined,
): string {
  if (status === "running") return tokens.accent;
  if (status === "failed" || status === "interrupted") return tokens.danger;
  if (!enabled) return tokens.ink4;
  return tokens.success;
}

interface ActionProps {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: React.ReactNode;
}

function RowAction({ label, onPress, disabled, danger, children }: ActionProps) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.action,
        (pressed || hovered) && { backgroundColor: hexToRgba(danger ? tokens.danger : tokens.accent, 0.12) },
        disabled && styles.disabled,
      ]}
    >
      {children}
    </Pressable>
  );
}

export interface ScheduleRowViewProps {
  row: ScheduleRow;
  onEdit: (row: ScheduleRow) => void;
  onNavigate?: () => void;
}

export function ScheduleRowView({ row, onEdit, onNavigate }: ScheduleRowViewProps) {
  const tokens = useTokens();
  const router = useRouter();
  const now = useClockSeconds() * 1000;
  const pause = usePauseSchedule();
  const resume = useResumeSchedule();
  const remove = useDeleteSchedule();
  const runNow = useRunScheduleNow();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const latest = row.latest_run;
  const running = latest?.status === "running";
  const busy = pause.isPending || resume.isPending || remove.isPending;
  const result = lastResultLine(latest, now);
  const failed = latest?.status === "failed" || latest?.status === "interrupted";
  const sessionId = latest?.session_id ?? null;

  return (
    <View style={[styles.row, { borderBottomColor: tokens.border }]}>
      <View style={styles.main}>
        <View style={[styles.dot, { backgroundColor: statusColor(tokens, row.enabled, latest?.status) }]} />
        <View style={styles.text}>
          <Text style={[typeScale.sub, { color: row.enabled ? tokens.ink : tokens.ink3 }]} numberOfLines={2}>
            {row.task}
          </Text>
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink2 }]} numberOfLines={2}>
            {cadenceLabel(row)}
            {"  ·  "}
            {nextRunLabel(row, now)}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
            {[formatCwd(row.cwd), row.model, row.mode].filter(Boolean).join(" · ")}
          </Text>
          {result ? (
            <Pressable
              disabled={!sessionId}
              onPress={() => {
                if (!sessionId) return;
                onNavigate?.();
                router.push(`/session/${sessionId}`);
              }}
              accessibilityRole={sessionId ? "link" : "text"}
              accessibilityLabel={sessionId ? `Open the session from the last run of ${row.task}` : undefined}
            >
              <Text
                style={[
                  typeScale.monoMeta,
                  { color: failed ? tokens.danger : tokens.ink3 },
                  sessionId ? styles.link : null,
                ]}
                numberOfLines={2}
              >
                {result}
              </Text>
            </Pressable>
          ) : null}
        </View>
      </View>
      <View style={styles.actions}>
        <RowAction
          label={`Run ${row.task} now`}
          onPress={() => runNow.mutate(row.id)}
          disabled={running || runNow.isPending}
        >
          <Play size={13} strokeWidth={1.75} color={tokens.ink2} />
        </RowAction>
        <RowAction label={`Edit ${row.task}`} onPress={() => onEdit(row)}>
          <Pencil size={13} strokeWidth={1.75} color={tokens.ink2} />
        </RowAction>
        <RowAction
          label={row.enabled ? `Pause ${row.task}` : `Resume ${row.task}`}
          onPress={() => (row.enabled ? pause.mutate(row.id) : resume.mutate(row.id))}
          disabled={busy}
        >
          {row.enabled ? (
            <Pause size={13} strokeWidth={1.75} color={tokens.ink2} />
          ) : (
            <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>resume</Text>
          )}
        </RowAction>
        <RowAction label={`Delete ${row.task}`} onPress={() => setConfirmDelete(true)} disabled={busy} danger>
          <Trash2 size={13} strokeWidth={1.75} color={tokens.ink3} />
        </RowAction>
      </View>
      {runNow.isError ? (
        <Text style={[typeScale.monoMeta, { color: tokens.danger }]}>
          {runNow.error instanceof Error ? runNow.error.message : "Could not start the run."}
        </Text>
      ) : null}
      <ConfirmDialog
        visible={confirmDelete}
        title="Delete this schedule?"
        message="Its timer is removed and its run history is deleted. Sessions it already created stay."
        confirmLabel="Delete"
        destructive
        loading={remove.isPending}
        onCancel={() => setConfirmDelete(false)}
        onConfirm={() => remove.mutate(row.id, { onSettled: () => setConfirmDelete(false) })}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  row: { paddingVertical: space.space8, gap: space.space4, borderBottomWidth: StyleSheet.hairlineWidth },
  main: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  dot: { width: 6, height: 6, borderRadius: 3, marginTop: 7 },
  text: { flex: 1, minWidth: 0, gap: 2 },
  link: { textDecorationLine: "underline" },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: space.space4 },
  action: {
    minWidth: 28,
    height: 28,
    paddingHorizontal: 6,
    borderRadius: radii.radius4,
    alignItems: "center",
    justifyContent: "center",
  },
  disabled: { opacity: 0.4 },
});
