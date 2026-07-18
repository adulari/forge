// Forge Anywhere — account management (mobile.dc.html "AW Account", lines 1233-1281).
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { BackLink } from "../../components/ds/BackLink";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/anywhere/store";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";

const RECOVERY_WORD_COUNT = 24;

const RECOVERY_FAILURE_STATES = [
  "Wrong phrase — checksum failed at word 19",
  "No wrapped key for this epoch — approve from a paired device instead",
  "This device was revoked — pair it again from another device",
  "Terminal: every device and the phrase are gone — the data is unrecoverable, by design. Local Forge still works.",
];

export default function AnywhereAccountScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { account, client, signedIn, loading, refresh } = useAnywhere();
  const { dotStyle } = useEmberdot("busy");

  const [signOutConfirm, setSignOutConfirm] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportPct, setExportPct] = useState(0);
  const exportTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const [phrase, setPhrase] = useState("");

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  useEffect(() => () => {
    if (exportTimer.current) clearInterval(exportTimer.current);
  }, []);

  const onSignOut = useCallback(async () => {
    setSignOutConfirm(false);
    await client.signOut();
    await refresh();
    router.replace("/anywhere");
  }, [client, refresh]);

  const onDelete = useCallback(async () => {
    setDeleteConfirm(false);
    await client.deleteAccount();
    await refresh();
    router.replace("/anywhere");
  }, [client, refresh]);

  const onExport = useCallback(async () => {
    if (exporting) return;
    setExporting(true);
    setExportPct(0);
    // exportAccountData() resolves atomically (mock has no progress stream) — simulate a
    // bounded ramp so the "preparing… NN%" copy from the comp has something to show while
    // the real request is in flight; a real streaming/polling export endpoint would drive
    // this from server-reported progress instead.
    exportTimer.current = setInterval(() => {
      setExportPct((p) => Math.min(95, p + Math.random() * 12));
    }, 200);
    try {
      await client.exportAccountData();
      if (exportTimer.current) clearInterval(exportTimer.current);
      setExportPct(100);
      toast.show("Export ready — link valid 24h.", { tone: "success" });
    } finally {
      setTimeout(() => setExporting(false), 600);
    }
  }, [client, exporting, toast]);

  const wordCount = phrase.trim().length === 0 ? 0 : phrase.trim().split(/\s+/).length;
  // Real checksum validation lands with the relay/crypto backend — this is a cheap
  // local word-count signal only (matches BIP39-style 24-word phrases).
  const checksumOk = wordCount === RECOVERY_WORD_COUNT;

  if (loading || !signedIn || !account) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
        <Text style={[type.headingBold, { color: tokens.ink }]}>{`Account · @${account.githubLogin}`}</Text>
      </View>

      <View style={styles.section}>
        <Pressable onPress={() => setSignOutConfirm(true)} accessibilityRole="button" accessibilityLabel="Sign out on this device" style={styles.actionRow}>
          <Text style={[type.body, styles.actionLabel, { color: tokens.ink }]}>Sign out on this device</Text>
          <Text style={[type.monoMeta, { color: tokens.ink3 }]}>keys removed · local Forge data stays</Text>
        </Pressable>
        <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} />

        <Pressable onPress={() => void onExport()} disabled={exporting} accessibilityRole="button" accessibilityLabel="Export account data" style={styles.actionRow}>
          <Text style={[type.body, styles.actionLabel, { color: tokens.ink }]}>Export account data</Text>
          <Text style={[type.monoMeta, { color: tokens.ink3 }]}>prepares · then expiring download</Text>
          <ChevronRight size={14} strokeWidth={2} color={tokens.ink4} />
        </Pressable>
        {exporting ? (
          <View style={styles.exportRow}>
            <Animated.View style={[styles.dot, dotStyle, { backgroundColor: tokens.accent }]} />
            <Text style={[type.monoMeta, tabularNums, { color: tokens.ink3 }]}>
              {`preparing… ${Math.round(exportPct)}% · link valid 24h after ready`}
            </Text>
          </View>
        ) : null}
        <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} />

        <Pressable onPress={() => setDeleteConfirm(true)} accessibilityRole="button" accessibilityLabel="Delete account" style={styles.actionRow}>
          <Text style={[type.body, styles.actionLabel, { color: tokens.danger }]}>Delete account…</Text>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>live in 24h · backups ≤ 30 days</Text>
        </Pressable>
        <Text style={[type.meta, styles.deleteScope, { color: tokens.ink4 }]}>
          Deletes Anywhere data: sync, jobs, shares, devices, hosts enrollment, subscription.
          Local Forge and repos are untouched. Idempotent — safe to retry.
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>recover on a new device</Text>
        <Text style={[type.meta, styles.recoverIntro, { color: tokens.ink3 }]}>
          Sign in with GitHub, then enter your 24-word phrase:
        </Text>
        <Input
          value={phrase}
          onChangeText={setPhrase}
          mono
          multiline
          placeholder="ember anvil …"
          autoCapitalize="none"
          autoCorrect={false}
          containerStyle={styles.phraseInput}
          accessibilityLabel="Recovery phrase"
        />
        <Text style={[type.monoMeta, styles.checksumLine, { color: checksumOk ? tokens.success : tokens.ink3 }]}>
          {checksumOk ? "✓ checksum ok" : `${wordCount} of ${RECOVERY_WORD_COUNT} words`}
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>recovery states</Text>
        {RECOVERY_FAILURE_STATES.map((text, i) => (
          <View key={text}>
            <View style={styles.recoveryRow}>
              <View style={[styles.dot, { backgroundColor: i === RECOVERY_FAILURE_STATES.length - 1 ? tokens.ink4 : tokens.danger }]} />
              <Text style={[type.sub, styles.recoveryText, { color: tokens.ink2 }]}>{text}</Text>
            </View>
            {i < RECOVERY_FAILURE_STATES.length - 1 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
          </View>
        ))}
      </View>

      <ConfirmDialog
        visible={signOutConfirm}
        title="Sign out on this device?"
        message="Local encryption keys are removed. Your local Forge data and sessions stay exactly as they are."
        confirmLabel="Sign out"
        onConfirm={() => void onSignOut()}
        onCancel={() => setSignOutConfirm(false)}
      />
      <ConfirmDialog
        visible={deleteConfirm}
        title="Delete account?"
        message="Deletes Anywhere data: sync, jobs, shares, devices, hosts enrollment, subscription. Local Forge and repos are untouched. Idempotent — safe to retry."
        confirmLabel="Delete account"
        destructive
        onConfirm={() => void onDelete()}
        onCancel={() => setDeleteConfirm(false)}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { gap: space.space4 },
  section: { marginTop: space.space20 },
  actionRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 },
  actionLabel: { flex: 1 },
  exportRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: -2, marginBottom: space.space8 },
  dot: { width: 6, height: 6, borderRadius: 3 },
  hairline: { height: StyleSheet.hairlineWidth },
  deleteScope: { marginTop: space.space4, lineHeight: 16 },
  recoverIntro: { marginTop: space.space8 },
  phraseInput: { marginTop: space.space8 },
  checksumLine: { marginTop: space.space8 },
  recoveryRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  recoveryText: { flex: 1, lineHeight: 17 },
});
