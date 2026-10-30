import { Inbox } from "lucide-react-native";
import { useRouter } from "expo-router";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { ScheduleRow, ScheduleRun } from "../../../lib/api";
import { useMarkScheduleRunRead, useScheduleRuns } from "../../../lib/queries";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { formatRelativeTime, type as typeScale } from "../../../theme/typography";
import { Button } from "../../ds/Button";
import { EmptyState } from "../../ds/EmptyState";
import { useClockSeconds } from "../../ds/relativeClock";
import { epochMs, RUN_STATUS_LABEL } from "./scheduleModel";

/** A run belongs in the inbox once it has an outcome worth reading; ticks that were skipped or are
 * still going are not results yet. */
export function isInboxRun(run: ScheduleRun): boolean {
  return run.status === "ok" || run.status === "failed" || run.status === "interrupted";
}

export function unreadCount(runs: ScheduleRun[] | undefined): number {
  return (runs ?? []).filter((run) => isInboxRun(run) && !run.read).length;
}

export interface ScheduleResultsProps {
  schedules: ScheduleRow[];
  onNavigate?: () => void;
}

export function ScheduleResults({ schedules, onNavigate }: ScheduleResultsProps) {
  const tokens = useTokens();
  const router = useRouter();
  const now = useClockSeconds() * 1000;
  const runsQuery = useScheduleRuns();
  const markRead = useMarkScheduleRunRead();
  const runs = (runsQuery.data ?? []).filter(isInboxRun);
  const unread = unreadCount(runsQuery.data);
  const taskOf = (id: string) => schedules.find((row) => row.id === id)?.task ?? "Deleted schedule";

  if (runs.length === 0) {
    return <EmptyState icon={Inbox} message="No results yet - each run's outcome lands here, unread until you open it." />;
  }

  return (
    <View style={styles.list}>
      <View style={styles.toolbar}>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{unread > 0 ? `${unread} unread` : "All caught up"}</Text>
        <Button
          label="Mark all read"
          variant="ghost"
          disabled={unread === 0}
          onPress={() => markRead.mutate(null)}
        />
      </View>
      {runs.map((run) => {
        const bad = run.status !== "ok";
        const detail = (run.status === "ok" ? run.summary : (run.error ?? run.summary)) ?? "";
        const when = formatRelativeTime(epochMs(run.finished_at ?? run.started_at) ?? now, now);
        return (
          <Pressable
            key={run.id}
            accessibilityRole="button"
            accessibilityLabel={`${RUN_STATUS_LABEL[run.status]}: ${taskOf(run.schedule_id)}${run.read ? "" : ", unread"}`}
            onPress={() => {
              if (!run.read) markRead.mutate(run.id);
              if (run.session_id) {
                onNavigate?.();
                router.push(`/session/${run.session_id}`);
              }
            }}
            style={[styles.row, { borderBottomColor: tokens.border }]}
          >
            <View
              style={[
                styles.dot,
                { backgroundColor: run.read ? "transparent" : bad ? tokens.danger : tokens.accent, borderColor: bad ? tokens.danger : tokens.ink4 },
              ]}
            />
            <View style={styles.text}>
              <Text
                style={[typeScale.sub, { color: tokens.ink, fontWeight: run.read ? "400" : "600" }]}
                numberOfLines={1}
              >
                {taskOf(run.schedule_id)}
              </Text>
              <Text style={[typeScale.monoMeta, { color: bad ? tokens.danger : tokens.ink3 }]} numberOfLines={1}>
                {RUN_STATUS_LABEL[run.status]} {when} ago{run.trigger === "manual" ? " · run now" : ""}
                {run.attempt > 1 ? ` · attempt ${run.attempt}` : ""}
              </Text>
              {detail ? (
                <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={3}>
                  {detail}
                </Text>
              ) : null}
            </View>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: { paddingVertical: space.space4 },
  toolbar: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingBottom: space.space4 },
  row: { flexDirection: "row", gap: space.space8, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  dot: { width: 7, height: 7, borderRadius: 4, marginTop: 6, borderWidth: StyleSheet.hairlineWidth },
  text: { flex: 1, minWidth: 0, gap: 2 },
});
