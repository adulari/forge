// Machined desktop shell — "Schedules" (D Schedules, docs/design/machined INVENTORY.md
// L395-403). Backed by the daemon's real schedule store (`/api/schedules`, crates/forge-cli/
// src/serve_schedules.rs): one dense row per schedule with a live/paused dot, the stored spec
// in mono, and pause/resume/delete. Reached from the command palette's Actions group.
import { Clock, Trash2 } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { type CreateScheduleRequest, type ScheduleRow } from "../../lib/api";
import { nextCronRun } from "../../lib/cron";
import {
  useCreateSchedule,
  useDeleteSchedule,
  usePauseSchedule,
  useResumeSchedule,
  useSchedules,
} from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { formatCwd, formatRelativeTime, tabularNums, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { Input } from "../ds/Input";
import { useRelativeClock } from "../ds/relativeClock";
import { Sheet } from "../ds/Sheet";

/** The daemon serialises timestamps in seconds on some rows and milliseconds on others; a
 * value below year-2001-in-ms can only be seconds. */
function epochMs(value: number | null): number | null {
  if (value == null || value <= 0) return null;
  return value < 1e12 ? value * 1000 : value;
}

/**
 * Next fire time for a stored spec: `every:<seconds>` and `daily:HH:MM` are arithmetic, and
 * `cron:<expr>` goes through the 5-field parser in lib/cron. `cron:` is a shared prefix for two
 * dialects (crates/forge-cli/src/cli/commands/schedule.rs): standard 5-field cron, which the daemon
 * translates into the host's native trigger — the same grammar lib/cron implements, so what we
 * label here is what the OS timer fires — and systemd's own `OnCalendar=` (`Mon *-*-* 09:00:00`),
 * kept for schedules created before that translation existed. Only the first parses here; an
 * OnCalendar row returns null and falls back to showing the last run, because a guessed "next"
 * would be worse than none. `spec_label` names which dialect the daemon recognised.
 */
function nextRunMs(row: ScheduleRow, now: number): number | null {
  if (!row.enabled) return null;

  if (row.cron.startsWith("every:")) {
    const seconds = parseInt(row.cron.slice("every:".length), 10);
    if (!Number.isFinite(seconds) || seconds <= 0) return null;
    const period = seconds * 1000;
    const base = epochMs(row.last_run) ?? epochMs(row.created_at) ?? now;
    return base + Math.max(1, Math.ceil((now - base) / period)) * period;
  }

  if (row.cron.startsWith("daily:")) {
    const [hours, minutes] = row.cron.slice("daily:".length).split(":").map((part) => parseInt(part, 10));
    if (!Number.isFinite(hours) || !Number.isFinite(minutes)) return null;
    const next = new Date(now);
    next.setHours(hours, minutes, 0, 0);
    if (next.getTime() <= now) next.setDate(next.getDate() + 1);
    return next.getTime();
  }

  if (row.cron.startsWith("cron:")) {
    return nextCronRun(row.cron.slice("cron:".length), now);
  }

  return null;
}

function trailingLabel(row: ScheduleRow, now: number = Date.now()): string {
  if (!row.enabled) return "paused";
  const next = nextRunMs(row, now);
  if (next != null) {
    const sameDay = new Date(next).toDateString() === new Date(now).toDateString();
    return sameDay || next - now < 24 * 60 * 60 * 1000
      ? `next ${new Date(next).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", hour12: false })}`
      : `next ${new Date(next).toLocaleDateString("en", { weekday: "short" })}`;
  }
  const last = epochMs(row.last_run);
  return last ? `last ${formatRelativeTime(last, now)} ago` : "not run yet";
}

/** Accepts the same spec strings `forge schedule` stores (`every:1800`, `daily:09:00`,
 * `cron:0 2 * * *`) plus the bare forms, and maps them onto the create endpoint's one-of. Anything
 * unrecognised goes to `cron`, which the daemon reads as standard cron first and as an
 * `OnCalendar=` expression second — so both dialects can be typed here. */
function parseCadence(raw: string): Pick<CreateScheduleRequest, "every" | "at" | "cron"> | null {
  const value = raw.trim();
  if (!value) return null;
  // The stored form counts seconds (`every:1800`) while the endpoint wants the `--every` shorthand
  // (`30m`), so a bare number is spelled out as seconds rather than sent as-is and rejected.
  const every = (interval: string) => ({ every: /^\d+$/.test(interval) ? `${interval}s` : interval });
  if (value.startsWith("every:")) return every(value.slice("every:".length));
  if (value.startsWith("daily:")) return { at: value.slice("daily:".length) };
  if (value.startsWith("at:")) return { at: value.slice("at:".length) };
  if (value.startsWith("cron:")) return { cron: value.slice("cron:".length) };
  if (/^\d+[smhd]?$/.test(value)) return every(value);
  if (/^\d{1,2}:\d{2}$/.test(value)) return { at: value };
  return { cron: value };
}

function ScheduleRowView({ row }: { row: ScheduleRow }) {
  const tokens = useTokens();
  // Shared 30s tick — the "next 02:00" / "last 4m ago" labels stay honest without a timer per row.
  useRelativeClock();
  const pause = usePauseSchedule();
  const resume = useResumeSchedule();
  const remove = useDeleteSchedule();
  const busy = pause.isPending || resume.isPending || remove.isPending;

  return (
    <View style={styles.row}>
      <View style={[styles.dot, { backgroundColor: row.enabled ? tokens.success : tokens.ink4 }]} />
      <View style={styles.rowText}>
        <Text style={[typeScale.sub, { color: row.enabled ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>
          {row.task}
        </Text>
        <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
          {[row.spec_label, formatCwd(row.cwd), row.model, row.mode].filter(Boolean).join(" · ")}
        </Text>
      </View>
      <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{trailingLabel(row)}</Text>
      <Pressable
        onPress={() => (row.enabled ? pause.mutate(row.id) : resume.mutate(row.id))}
        disabled={busy}
        accessibilityRole="button"
        accessibilityLabel={row.enabled ? `Pause ${row.task}` : `Resume ${row.task}`}
        style={({ pressed }) => [
          styles.rowAction,
          { borderColor: tokens.border },
          pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
          busy && styles.disabled,
        ]}
      >
        <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>{row.enabled ? "pause" : "resume"}</Text>
      </Pressable>
      <Pressable
        onPress={() => remove.mutate(row.id)}
        disabled={busy}
        accessibilityRole="button"
        accessibilityLabel={`Delete ${row.task}`}
        style={({ pressed }) => [styles.iconAction, pressed && { backgroundColor: hexToRgba(tokens.danger, 0.12) }, busy && styles.disabled]}
      >
        <Trash2 size={13} strokeWidth={1.75} color={tokens.ink3} />
      </Pressable>
    </View>
  );
}

function CreateScheduleForm({ onDone }: { onDone: () => void }) {
  const tokens = useTokens();
  const create = useCreateSchedule();
  const [task, setTask] = useState("");
  const [cadence, setCadence] = useState("");
  const parsed = parseCadence(cadence);
  const valid = task.trim().length > 0 && parsed != null;

  return (
    <View style={[styles.form, { borderColor: tokens.border }]}>
      <Input label="Task" value={task} onChangeText={setTask} placeholder="Run the dependency audit" />
      <Input
        label="Cadence"
        value={cadence}
        onChangeText={setCadence}
        mono
        placeholder="every:30m · daily:02:00 · 0 6 * * 1"
      />
      {/* Standard 5-field cron is translated into the host's native timer, so it is the form worth
          advertising; systemd OnCalendar strings still install, but only on a Linux daemon. */}
      <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>
        {"every:30m · daily:HH:MM · cron: minute hour day-of-month month day-of-week"}
      </Text>
      {create.isError ? (
        <Text style={[typeScale.monoMeta, { color: tokens.danger }]}>
          {create.error instanceof Error ? create.error.message : "Could not create the schedule."}
        </Text>
      ) : null}
      <View style={styles.formActions}>
        <Button label="Cancel" variant="ghost" onPress={onDone} />
        <Button
          label="Create"
          loading={create.isPending}
          disabled={!valid}
          onPress={() => {
            if (!valid || !parsed) return;
            create.mutate({ task: task.trim(), ...parsed }, { onSuccess: onDone });
          }}
        />
      </View>
    </View>
  );
}

export interface SchedulesPanelProps {
  visible: boolean;
  onClose: () => void;
}

export function SchedulesPanel({ visible, onClose }: SchedulesPanelProps) {
  const tokens = useTokens();
  const query = useSchedules();
  const [creating, setCreating] = useState(false);
  const rows = query.data ?? [];

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Schedules">
      <View style={styles.content}>
        <View style={styles.head}>
          <Text style={[typeScale.headingBold, styles.headTitle, { color: tokens.ink }]}>Schedules</Text>
          <Pressable
            onPress={() => setCreating((open) => !open)}
            accessibilityRole="button"
            accessibilityLabel="New schedule"
            accessibilityState={{ expanded: creating }}
            style={({ pressed }) => [
              styles.rowAction,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>+ New schedule</Text>
          </Pressable>
        </View>

        {creating ? <CreateScheduleForm onDone={() => setCreating(false)} /> : null}

        {query.isLoading && rows.length === 0 ? (
          <Text style={[typeScale.sub, styles.loading, { color: tokens.ink3 }]}>Loading schedules…</Text>
        ) : rows.length === 0 ? (
          <EmptyState icon={Clock} message="No schedules yet — a recurring run you create here shows up in this list." />
        ) : (
          <View style={styles.list}>
            {rows.map((row) => (
              <ScheduleRowView key={row.id} row={row} />
            ))}
          </View>
        )}

        <Text style={[typeScale.monoMeta, styles.footer, { color: tokens.ink4, borderTopColor: tokens.border }]}>
          Runs land in Fleet as ordinary sessions
        </Text>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space8 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headTitle: { flex: 1 },
  list: { paddingVertical: space.space4 },
  // 30px dense desktop row (design's 8px/10px padding around a two-line block).
  row: { minHeight: 30, flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space4 },
  rowText: { flex: 1, minWidth: 0, gap: 1 },
  rowAction: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: 8, paddingVertical: 3 },
  iconAction: { width: 24, height: 24, borderRadius: radii.radius4, alignItems: "center", justifyContent: "center" },
  disabled: { opacity: 0.5 },
  dot: { width: 5, height: 5, borderRadius: 3 },
  form: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: space.space8 },
  formActions: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8 },
  loading: { paddingVertical: space.space16 },
  footer: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8 },
});
