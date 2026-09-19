// Forge Anywhere — Encrypted storage (mobile.dc.html "AW Storage" lines 1133-1183).
// Retention windows are fixed app policy (not per-account data — there's no client method
// for it), so `RETENTION_ROWS` below is a local constant built from the real `RetentionRow`
// shape (lib/anywhere/types.ts) rather than ad hoc strings.
import { router } from "expo-router";
import React, { useEffect } from "react";
import { StyleSheet, Text, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";

import { SettingsShell } from "../(tabs)/settings";
import { BackLink } from "../../components/ds/BackLink";
import { Banner, type BannerTone } from "../../components/ds/Banner";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Skeleton } from "../../components/ds/Skeleton";
import { goBackOr } from "../../lib/nav";
import { formatBytes } from "../../lib/anywhere/format";
import { useAnywhere } from "../../lib/anywhere/store";
import { useAnywhere as useEncryptedAnywhere } from "../../lib/AnywhereProvider";
import type { RetentionRow, StorageState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale, tabularNums } from "../../theme/typography";

const COUNTS_COPY =
  "Counts: synced sessions, messages, checkpoints, tool calls, usage, memories, settings, commands, skills, agents, workflows and portable file records — all sealed before upload.";

const RETENTION_ROWS: RetentionRow[] = [
  { kind: "capsules", label: "Handoff capsules & temp blobs", windowDays: 7 },
  { kind: "superseded-revisions", label: "Superseded revisions", windowDays: 30 },
  { kind: "tombstones", label: "Deletion tombstones", windowDays: 90 },
  { kind: "shares", label: "Replay shares", windowDays: null },
  { kind: "post-subscription", label: "After subscription ends", windowDays: 90 },
];

function retentionValue(row: RetentionRow): string {
  if (row.kind === "shares") return "until expiry/revoke";
  if (row.kind === "post-subscription") return `${row.windowDays} days, then deleted`;
  return `${row.windowDays} days`;
}

interface StorageBannerInfo {
  tone: BannerTone;
  message: string;
  actionLabel?: string;
}

function storageFigures(usedBytes: number, quotaBytes: number): { usedBytes: number; quotaBytes: number; state: StorageState } {
  const fraction = quotaBytes > 0 ? usedBytes / quotaBytes : 0;
  const state: StorageState = fraction >= 1 ? "full" : fraction >= 0.9 ? "nearly-full" : "ok";
  return { usedBytes, quotaBytes, state };
}

function storageBannerInfo(state: StorageState, usedBytes: number, quotaBytes: number): StorageBannerInfo | null {
  switch (state) {
    case "ok":
      return null;
    case "calculating":
      return { tone: "neutral", message: "Calculating usage…" };
    case "nearly-full":
      return {
        tone: "warn",
        message: `${formatBytes(usedBytes)} of ${formatBytes(quotaBytes)} — nearly full`,
        actionLabel: "Free up space",
      };
    case "full":
      return { tone: "danger", message: "Full — new writes blocked. Download and delete still work." };
    case "stale-figure":
      return { tone: "neutral", message: "Figure is stale — showing the last computed value." };
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

export default function AnywhereStorageScreen() {
  const tokens = useTokens();
  const { signedIn, loading: accountLoading } = useAnywhere();
  // The figures come from the real account the Anywhere hub shows. This screen used to read the
  // prototype's MockAnywhereClient — a made-up usage figure and a "Clean up" preview offering to
  // free 0.3 GB, which then admitted it was not wired to anything.
  const real = useEncryptedAnywhere();
  const loading = real.account == null && real.phase !== "ready";
  const storage = real.account
    ? storageFigures(real.account.storage_used_bytes, real.account.storage_limit_bytes)
    : null;

  useEffect(() => {
    if (!accountLoading && !signedIn) router.replace("/anywhere");
  }, [accountLoading, signedIn]);

  if (!signedIn) return null;

  const pct = storage && storage.quotaBytes > 0 ? Math.round((storage.usedBytes / storage.quotaBytes) * 100) : 0;
  const bannerInfo = storage ? storageBannerInfo(storage.state, storage.usedBytes, storage.quotaBytes) : null;

  return (
    <SettingsShell active="anywhere">
      <Screen scroll contentContainerStyle={styles.content}>
        <View style={styles.headerRow}>
          <BackLink label="Anywhere" onPress={() => goBackOr("/anywhere")} />
          <Text style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>Encrypted storage</Text>
        </View>

        {loading || !storage ? (
          <View style={styles.skeletonWrap}>
            <Skeleton width={140} height={30} />
            <Skeleton width="100%" height={5} />
          </View>
        ) : (
          <View style={styles.section}>
            <View style={styles.figureRow}>
              <Text style={[typeScale.headingBold, styles.figure, tabularNums, { color: tokens.ink }]}>
                {formatBytes(storage.usedBytes)}
              </Text>
              <Text style={[typeScale.sub, tabularNums, { color: tokens.ink3 }]}>
                {`of ${formatBytes(storage.quotaBytes)} used · ${pct}%`}
              </Text>
            </View>
            <View style={[styles.gaugeTrack, { backgroundColor: tokens.border }]}>
              <LinearGradient
                colors={[tokens.accentPressed, tokens.accent]}
                start={{ x: 0, y: 0 }}
                end={{ x: 1, y: 0 }}
                style={[styles.gaugeFill, { width: `${Math.min(100, Math.max(0, pct))}%` }]}
              />
            </View>
            <Text style={[typeScale.meta, styles.countsText, { color: tokens.ink3 }]}>{COUNTS_COPY}</Text>
          </View>
        )}

        {bannerInfo ? (
          <Banner
            tone={bannerInfo.tone}
            message={bannerInfo.message}
          />
        ) : null}

        <View style={styles.section}>
          <SectionHeader>Retention</SectionHeader>
          {RETENTION_ROWS.map((row, index) => (
            <View key={row.kind}>
              <View style={styles.retentionRow}>
                <Text style={[typeScale.sub, styles.retentionLabel, { color: tokens.ink2 }]} numberOfLines={1}>
                  {row.label}
                </Text>
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{retentionValue(row)}</Text>
              </View>
              {index < RETENTION_ROWS.length - 1 ? (
                <View style={[styles.separator, { backgroundColor: tokens.hairline }]} />
              ) : null}
            </View>
          ))}
        </View>

      </Screen>

    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space4 },
  headerTitle: { flex: 1 },
  section: { marginTop: space.space20, paddingHorizontal: space.space4 },
  skeletonWrap: { marginTop: space.space16, paddingHorizontal: space.space4, gap: space.space8 },
  figureRow: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  figure: { fontSize: 26 },
  gaugeTrack: { height: 5, borderRadius: 3, overflow: "hidden", marginTop: space.space8 },
  gaugeFill: { height: "100%", borderRadius: 3 },
  countsText: { marginTop: space.space8, lineHeight: 16 },
  retentionRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  retentionLabel: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth },
  cleanupRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12, minHeight: 44 },
  cleanupLabel: { flex: 1 },
  sheetContent: { padding: space.space20, gap: space.space12 },
  previewText: { lineHeight: 19 },
  previewSkeleton: { marginTop: space.space4 },
  sheetActions: { flexDirection: "row", gap: space.space8, marginTop: space.space8 },
  sheetButton: { flex: 1 },
});
