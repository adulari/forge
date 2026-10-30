import { ChevronDown, ChevronRight } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { CreateScheduleRequest, ScheduleRow } from "../../../lib/api";
import { isKnownTimeZone, localTimeZone } from "../../../lib/cronZone";
import { useCreateSchedule, useUpdateSchedule } from "../../../lib/queries";
import {
  CADENCE_OPTIONS,
  cadenceFromStored,
  cadenceToBody,
  bodyToStored,
  defaultCadence,
  type Cadence,
} from "../../../lib/scheduleCadence";
import { useTokens } from "../../../theme/ThemeProvider";
import { radii, space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";
import { Button } from "../../ds/Button";
import { Chip } from "../../ds/Chip";
import { Input } from "../../ds/Input";
import { Segmented } from "../../ds/Segmented";
import { useClockSeconds } from "../../ds/relativeClock";
import { Switch } from "../../ds/Switch";
import { previewLabel, type ScheduleTemplate } from "./scheduleModel";

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const toInt = (raw: string): number | undefined => {
  const parsed = parseInt(raw.trim(), 10);
  return Number.isFinite(parsed) ? parsed : undefined;
};

export interface ScheduleFormProps {
  editing: ScheduleRow | null;
  template: ScheduleTemplate | null;
  defaultCwd?: string;
  onDone: () => void;
}

function initialCadence(editing: ScheduleRow | null, template: ScheduleTemplate | null): Cadence {
  if (editing) return cadenceFromStored(editing.cron, editing.max_iterations);
  const base = defaultCadence();
  if (!template) return base;
  return { ...base, kind: template.cadence.kind, time: template.cadence.time, weekday: template.cadence.weekday ?? 1 };
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  const tokens = useTokens();
  return (
    <View style={styles.field}>
      <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{label}</Text>
      {children}
    </View>
  );
}

export function ScheduleForm({ editing, template, defaultCwd, onDone }: ScheduleFormProps) {
  const tokens = useTokens();
  const create = useCreateSchedule();
  const update = useUpdateSchedule();
  const mutation = editing ? update : create;

  const [task, setTask] = useState(editing?.task ?? template?.task ?? "");
  const [cadence, setCadence] = useState<Cadence>(() => initialCadence(editing, template));
  const [cwd, setCwd] = useState(editing?.cwd ?? defaultCwd ?? "");
  const [tz, setTz] = useState(editing?.tz ?? "");
  const [advanced, setAdvanced] = useState(false);
  const [maxRuntimeMin, setMaxRuntimeMin] = useState(
    editing?.max_runtime_secs ? String(Math.round(editing.max_runtime_secs / 60)) : "",
  );
  const [maxRuns, setMaxRuns] = useState(editing?.max_iterations ? String(editing.max_iterations) : "");
  const [retries, setRetries] = useState(String(editing?.retries ?? 0));
  const [backoff, setBackoff] = useState(String(editing?.backoff_secs ?? 60));
  const [misfire, setMisfire] = useState<"run" | "skip">(editing?.misfire ?? "run");
  const [stopOnError, setStopOnError] = useState(editing?.stop_on_error ?? false);
  const [stopWhen, setStopWhen] = useState(editing?.stop_when ?? "");

  const patch = (change: Partial<Cadence>) => setCadence((current) => ({ ...current, ...change }));
  const compiled = cadenceToBody(cadence);
  const zone = tz.trim();
  const zoneError = zone && !isKnownTimeZone(zone) ? `"${zone}" is not a time zone name like Europe/Amsterdam` : undefined;
  const valid = task.trim().length > 0 && compiled != null && !zoneError;

  const now = useClockSeconds() * 1000;
  const stored = compiled ? bodyToStored(compiled.body) : null;
  const preview = stored ? previewLabel(stored, zone || null, now) : null;

  const submit = () => {
    if (!valid || !compiled) return;
    const body: CreateScheduleRequest = {
      task: task.trim(),
      ...compiled.body,
      ...(cwd.trim() ? { cwd: cwd.trim() } : {}),
      tz: zone,
      max_runtime_secs: (toInt(maxRuntimeMin) ?? 0) > 0 ? (toInt(maxRuntimeMin) as number) * 60 : undefined,
      max_iterations: compiled.maxIterations ?? ((toInt(maxRuns) ?? 0) > 0 ? toInt(maxRuns) : undefined),
      retries: Math.min(5, Math.max(0, toInt(retries) ?? 0)),
      backoff_secs: Math.min(3600, Math.max(0, toInt(backoff) ?? 60)),
      misfire,
      stop_on_error: stopOnError,
      stop_when: stopWhen.trim() || undefined,
      ...(editing?.mode ? { mode: editing.mode } : {}),
      ...(editing?.model ? { model: editing.model } : {}),
    };
    if (editing) update.mutate({ id: editing.id, body }, { onSuccess: onDone });
    else create.mutate(body, { onSuccess: onDone });
  };

  const machineZone = localTimeZone();

  return (
    <View style={[styles.form, { borderColor: tokens.border }]}>
      <Input
        label="What should Forge do?"
        value={task}
        onChangeText={setTask}
        placeholder="Run the dependency audit and list what to upgrade"
        multiline
        accessibilityLabel="Task"
      />

      <Field label="Repeats">
        <View style={styles.chips} accessibilityRole="radiogroup">
          {CADENCE_OPTIONS.map((option) => (
            <Chip
              key={option.value}
              label={option.label}
              size="compact"
              selected={cadence.kind === option.value}
              onPress={() => patch({ kind: option.value })}
            />
          ))}
        </View>
      </Field>

      {cadence.kind === "hourly" ? (
        <Input
          label="Minute past the hour"
          value={String(cadence.minute)}
          onChangeText={(text) => patch({ minute: toInt(text) ?? 0 })}
          keyboardType="number-pad"
          mono
        />
      ) : null}
      {cadence.kind === "weekly" ? (
        <Field label="On">
          <View style={styles.chips}>
            {WEEKDAYS.map((name, index) => (
              <Chip
                key={name}
                label={name}
                size="compact"
                selected={cadence.weekday === index}
                onPress={() => patch({ weekday: index })}
              />
            ))}
          </View>
        </Field>
      ) : null}
      {cadence.kind === "once" ? (
        <Input
          label="Date"
          value={cadence.date}
          onChangeText={(text) => patch({ date: text })}
          placeholder="YYYY-MM-DD"
          mono
        />
      ) : null}
      {cadence.kind === "daily" ||
      cadence.kind === "weekdays" ||
      cadence.kind === "weekly" ||
      cadence.kind === "once" ? (
        <Input
          label="At (24-hour)"
          value={cadence.time}
          onChangeText={(text) => patch({ time: text })}
          placeholder="09:00"
          mono
        />
      ) : null}
      {cadence.kind === "custom" ? (
        <Input
          label="Cron or interval"
          value={cadence.custom}
          onChangeText={(text) => patch({ custom: text })}
          placeholder="0 3 * * 1-5   ·   every:30m   ·   daily:02:00"
          mono
        />
      ) : null}

      <Input
        label={`Time zone${machineZone ? ` (blank = this machine, ${machineZone})` : ""}`}
        value={tz}
        onChangeText={setTz}
        placeholder="Europe/Amsterdam"
        error={zoneError}
        mono
      />
      <Input label="Folder (blank = daemon default)" value={cwd} onChangeText={setCwd} mono placeholder="/path/to/project" />

      <Text
        style={[typeScale.monoMeta, { color: preview ? tokens.ink2 : tokens.ink4 }]}
        accessibilityLiveRegion="polite"
      >
        {preview ?? "Pick a cadence to see when it runs"}
      </Text>

      <Pressable
        onPress={() => setAdvanced((open) => !open)}
        accessibilityRole="button"
        accessibilityState={{ expanded: advanced }}
        style={styles.disclosure}
      >
        {advanced ? (
          <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
        ) : (
          <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
        )}
        <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>Limits and behaviour</Text>
      </Pressable>
      {advanced ? (
        <View style={styles.advanced}>
          <View style={styles.pair}>
            <Input
              containerStyle={styles.half}
              label="Max runtime (min)"
              value={maxRuntimeMin}
              onChangeText={setMaxRuntimeMin}
              keyboardType="number-pad"
              placeholder="none"
              mono
            />
            <Input
              containerStyle={styles.half}
              label="Stop after (runs)"
              value={compiled?.maxIterations ? "1" : maxRuns}
              onChangeText={setMaxRuns}
              keyboardType="number-pad"
              placeholder="never"
              disabled={Boolean(compiled?.maxIterations)}
              mono
            />
          </View>
          <View style={styles.pair}>
            <Input
              containerStyle={styles.half}
              label="Retries (0-5)"
              value={retries}
              onChangeText={setRetries}
              keyboardType="number-pad"
              mono
            />
            <Input
              containerStyle={styles.half}
              label="Retry backoff (s)"
              value={backoff}
              onChangeText={setBackoff}
              keyboardType="number-pad"
              mono
            />
          </View>
          <Field label="If a run was missed while the machine was off">
            <Segmented
              options={[
                { value: "run", label: "Run once on wake" },
                { value: "skip", label: "Skip it" },
              ]}
              value={misfire}
              onChange={setMisfire}
            />
          </Field>
          <View style={styles.switchRow}>
            <Text style={[typeScale.sub, styles.switchLabel, { color: tokens.ink }]}>Pause after a failed run</Text>
            <Switch value={stopOnError} onValueChange={setStopOnError} accessibilityLabel="Pause after a failed run" />
          </View>
          <Input
            label="Stop when... (checked by the run itself)"
            value={stopWhen}
            onChangeText={setStopWhen}
            placeholder="CI on main is green again"
          />
        </View>
      ) : null}

      {mutation.isError ? (
        <Text style={[typeScale.monoMeta, { color: tokens.danger }]}>
          {mutation.error instanceof Error ? mutation.error.message : "Could not save the schedule."}
        </Text>
      ) : null}
      <View style={styles.actions}>
        <Button label="Cancel" variant="ghost" onPress={onDone} />
        <Button
          label={editing ? "Save" : "Create"}
          loading={mutation.isPending}
          disabled={!valid}
          onPress={submit}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  form: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: space.space12 },
  field: { gap: space.space4 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space4 },
  disclosure: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingVertical: space.space4 },
  advanced: { gap: space.space12 },
  pair: { flexDirection: "row", gap: space.space8 },
  half: { flex: 1, minWidth: 0 },
  switchRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  switchLabel: { flex: 1 },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8 },
});
