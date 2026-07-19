// Forge Anywhere — remote jobs queue management (mobile.dc.html "AW Remote Jobs",
// lines 959-1029). Management only — running jobs themselves live in Fleet.
import { router } from "expo-router";
import { Check, ChevronRight } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { BackLink } from "../../components/ds/BackLink";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere, useAnywhereJobs } from "../../lib/anywhere/store";
import type { JobState, RemoteJob } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";
import type { ColorTokens } from "../../theme/tokens";

interface JobPresentation {
  dotColor: keyof Pick<ColorTokens, "accent" | "warn" | "danger" | "ink2" | "ink4">;
  pulsing: boolean;
  checkGlyph: boolean;
  metaColor: keyof Pick<ColorTokens, "ink2" | "ink3" | "warn" | "danger" | "success" | "ink4">;
  metaText: (job: RemoteJob) => string;
  titleDim: boolean;
  strikethrough: boolean;
}

function presentation(state: JobState): JobPresentation {
  switch (state) {
    case "running-on-host":
      return { dotColor: "accent", pulsing: true, checkGlyph: false, metaColor: "ink3", metaText: (j) => `running on ${j.hostName}`, titleDim: false, strikethrough: false };
    case "waiting-for-host":
      return { dotColor: "warn", pulsing: true, checkGlyph: false, metaColor: "warn", metaText: (j) => `waiting for ${j.hostName}`, titleDim: false, strikethrough: false };
    case "uploaded-sealed":
      return { dotColor: "ink2", pulsing: false, checkGlyph: false, metaColor: "ink3", metaText: () => "uploaded · sealed", titleDim: true, strikethrough: false };
    case "queued-locally-offline":
      return { dotColor: "ink4", pulsing: false, checkGlyph: false, metaColor: "ink4", metaText: () => "queued locally · offline", titleDim: true, strikethrough: false };
    case "completed":
      return {
        dotColor: "ink4",
        pulsing: false,
        checkGlyph: true,
        metaColor: "success",
        metaText: (j) => `completed ${new Date(j.updatedAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false })}`,
        titleDim: true,
        strikethrough: false,
      };
    case "failed":
      return { dotColor: "danger", pulsing: false, checkGlyph: false, metaColor: "danger", metaText: () => "failed · exit 1", titleDim: true, strikethrough: false };
    case "expired-after-7d-unclaimed":
      return { dotColor: "ink4", pulsing: false, checkGlyph: false, metaColor: "ink4", metaText: () => "expired after 7d unclaimed", titleDim: true, strikethrough: true };
    case "blocked-read-only-plan":
      return { dotColor: "danger", pulsing: false, checkGlyph: false, metaColor: "danger", metaText: () => "blocked · read-only plan", titleDim: true, strikethrough: false };
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

function JobDot({ colorKey, pulsing, checkGlyph }: { colorKey: JobPresentation["dotColor"]; pulsing: boolean; checkGlyph: boolean }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(pulsing ? "busy" : "idle");
  if (checkGlyph) {
    return (
      <View style={[styles.checkGlyph, { backgroundColor: tokens.successBg }]}>
        <Check size={9} strokeWidth={3} color={tokens.success} />
      </View>
    );
  }
  return <Animated.View style={[styles.dot, pulsing ? dotStyle : undefined, { backgroundColor: tokens[colorKey] }]} />;
}

function JobRow({ job, onCancel, onReplace, onRequeue, showSeparator }: {
  job: RemoteJob;
  onCancel: (job: RemoteJob) => void;
  onReplace: (job: RemoteJob) => void;
  onRequeue: (job: RemoteJob) => void;
  showSeparator: boolean;
}) {
  const tokens = useTokens();
  const p = presentation(job.state);

  return (
    <View>
      <View style={styles.row}>
        <JobDot colorKey={p.dotColor} pulsing={p.pulsing} checkGlyph={p.checkGlyph} />
        <Text
          style={[
            type.bodyBold,
            styles.title,
            { color: p.titleDim ? tokens.ink2 : tokens.ink },
            p.strikethrough ? styles.strike : undefined,
          ]}
          numberOfLines={1}
        >
          {job.sessionTitle}
        </Text>
        <Text style={[type.monoMeta, tabularNums, { color: tokens[p.metaColor] }]} numberOfLines={1}>
          {p.metaText(job)}
        </Text>
        {job.state === "uploaded-sealed" ? (
          <>
            <Pressable onPress={() => onCancel(job)} accessibilityRole="button" accessibilityLabel={`Cancel ${job.sessionTitle}`} hitSlop={6}>
              <Text style={[type.meta, { color: tokens.ink2 }]}>Cancel</Text>
            </Pressable>
            <Pressable onPress={() => onReplace(job)} accessibilityRole="button" accessibilityLabel={`Replace ${job.sessionTitle}`} hitSlop={6}>
              <Text style={[type.meta, { color: tokens.accent }]}>Replace…</Text>
            </Pressable>
          </>
        ) : null}
        {job.state === "failed" ? (
          <Pressable onPress={() => onRequeue(job)} accessibilityRole="button" accessibilityLabel={`Requeue ${job.sessionTitle}`} hitSlop={6}>
            <Text style={[type.meta, { color: tokens.accent }]}>Requeue</Text>
          </Pressable>
        ) : null}
        {job.state === "blocked-read-only-plan" ? (
          <Pressable onPress={() => router.push("/anywhere/billing")} accessibilityRole="button" accessibilityLabel="Open billing" hitSlop={6}>
            <Text style={[type.meta, { color: tokens.accent }]}>Billing</Text>
          </Pressable>
        ) : null}
        {job.state === "running-on-host" ? <ChevronRight size={13} strokeWidth={2} color={tokens.ink4} /> : null}
      </View>
      {showSeparator ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

export default function AnywhereJobsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { signedIn, loading, client } = useAnywhere();
  const { jobs, refresh } = useAnywhereJobs();

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  const onCancel = useCallback(
    async (job: RemoteJob) => {
      await client.cancelJob(job.id);
      await refresh();
      toast.show("Job cancelled.");
    },
    [client, refresh, toast],
  );

  const onReplace = useCallback(
    async (job: RemoteJob) => {
      await client.cancelJob(job.id);
      await refresh();
      toast.show("Job cancelled — start a new session on the same host.");
      router.push("/new-session");
    },
    [client, refresh, toast],
  );

  const onRequeue = useCallback(
    async (job: RemoteJob) => {
      await client.requeueJob(job.id);
      await refresh();
      toast.show("Requeued.");
    },
    [client, refresh, toast],
  );

  const sorted = useMemo(() => [...jobs].sort((a, b) => b.updatedAt - a.updatedAt), [jobs]);

  if (loading || !signedIn) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
        <View style={styles.headerRow}>
          <Text style={[type.headingBold, styles.headerTitle, { color: tokens.ink }]}>Remote jobs</Text>
          <Text style={[type.monoMeta, { color: tokens.ink3 }]}>queue history</Text>
        </View>
      </View>
      <Text style={[type.meta, styles.note, { color: tokens.ink3 }]}>
        Running jobs live in Fleet as ordinary sessions. This queue is management only.
      </Text>

      <View style={styles.list}>
        {sorted.map((job, i) => (
          <JobRow
            key={job.id}
            job={job}
            onCancel={onCancel}
            onReplace={onReplace}
            onRequeue={onRequeue}
            showSeparator={i < sorted.length - 1}
          />
        ))}
      </View>

      <Text style={[type.meta, styles.legal, { color: tokens.ink4 }]}>
        Queued jobs are immutable — before claim you can Cancel, or Replace (cancel + prefilled
        composer → new sealed job). After claim, control moves to the session itself. Job
        detail: title and working directory are end-to-end encrypted; the relay stores routing
        metadata — host id, size, kind, timestamps, signature.
      </Text>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { gap: space.space4 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headerTitle: { flex: 1 },
  note: { marginLeft: space.space4, lineHeight: 17 },
  list: { marginTop: space.space16 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 },
  title: { flex: 1 },
  strike: { textDecorationLine: "line-through" },
  dot: { width: 7, height: 7, borderRadius: 3.5 },
  checkGlyph: { width: 16, height: 16, borderRadius: 8, alignItems: "center", justifyContent: "center" },
  hairline: { height: StyleSheet.hairlineWidth, marginLeft: 17 },
  legal: { marginTop: space.space16, lineHeight: 17 },
});
