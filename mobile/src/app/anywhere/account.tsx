// Forge Anywhere — Account (mobile.dc.html "AW Billing Storage Account" lines
// 596-602, desktop.dc.html "D AW Settings" account row lines 1019). Real data only:
// sign-out and the clean-reset ("delete account") flow both call the actual
// AnywhereProvider methods already used by recovery-phrase.tsx's reset panel — this
// screen just gives that same real flow its own dedicated route. "Export account
// data" appears in the comp but AnywhereContextValue has no export endpoint yet, so
// it is intentionally omitted rather than wired to a fake result.
import { Redirect, router } from "expo-router";
import { Trash2 } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export default function AnywhereAccountScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [signingOut, setSigningOut] = useState(false);
  const [showReset, setShowReset] = useState(false);
  const [resetConfirmation, setResetConfirmation] = useState("");
  const [busy, setBusy] = useState(false);

  const signOut = useCallback(async () => {
    setSigningOut(true);
    try {
      await anywhere.logout();
    } finally {
      setSigningOut(false);
    }
  }, [anywhere]);

  const scheduleReset = useCallback(async () => {
    setBusy(true);
    try {
      const executeAt = await anywhere.scheduleCleanReset(resetConfirmation);
      toast.show(`Reset scheduled for ${new Date(executeAt).toLocaleString()}.`, { tone: "neutral" });
      setShowReset(false);
      setResetConfirmation("");
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Reset could not be scheduled.", { tone: "danger" });
    } finally {
      setBusy(false);
    }
  }, [anywhere, resetConfirmation, toast]);

  const cancelReset = useCallback(async () => {
    setBusy(true);
    try {
      await anywhere.cancelCleanReset();
      toast.show("Clean reset canceled.", { tone: "neutral" });
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Reset could not be canceled.", { tone: "danger" });
    } finally {
      setBusy(false);
    }
  }, [anywhere, toast]);

  if (anywhere.phase !== "ready") return <Redirect href="/anywhere" />;

  const pendingReset = anywhere.account?.pending_reset ?? null;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <View style={styles.header}>
          <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
          <Text accessibilityRole="header" style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>
            {`Account${anywhere.credentials?.githubLogin ? ` · @${anywhere.credentials.githubLogin}` : ""}`}
          </Text>
        </View>

        {pendingReset ? (
          <Banner
            tone="danger"
            message={`Clean reset scheduled for ${new Date(pendingReset.executes_at_ms).toLocaleString()}.`}
            actionLabel={pendingReset.cancelable ? "Cancel reset" : undefined}
            onAction={pendingReset.cancelable ? () => void cancelReset() : undefined}
          />
        ) : null}

        <SectionHeader>account</SectionHeader>
        <Pressable
          onPress={() => void signOut()}
          disabled={signingOut}
          accessibilityRole="button"
          accessibilityLabel="Sign out on this device"
          style={[styles.row, { borderColor: tokens.border }]}
        >
          <Text style={[typeScale.body, styles.rowLabel, { color: tokens.ink2 }]}>Sign out on this device</Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>keys removed · local data stays</Text>
        </Pressable>

        {!showReset ? (
          <Pressable
            onPress={() => setShowReset(true)}
            accessibilityRole="button"
            accessibilityLabel="Delete account"
            style={[styles.row, styles.dangerRow, { borderColor: hexToRgba(tokens.danger, 0.35) }]}
          >
            <Text style={[typeScale.body, styles.rowLabel, { color: tokens.danger }]}>Delete account…</Text>
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>live in 7 days · cancelable</Text>
          </Pressable>
        ) : (
          <View style={[styles.resetPanel, { borderColor: tokens.danger, backgroundColor: tokens.dangerBg }]}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Schedule a clean reset</Text>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              This waits seven days, then permanently deletes hosted encrypted account data. It never deletes local
              Forge data. Enrolled devices are notified and can cancel.
            </Text>
            <Input
              label="Type DELETE MY FORGE ANYWHERE DATA"
              value={resetConfirmation}
              onChangeText={setResetConfirmation}
              autoCapitalize="characters"
              autoCorrect={false}
              accessibilityHint="Exact destructive confirmation phrase"
            />
            <View style={styles.actionRow}>
              <Button
                label="Keep account"
                variant="ghost"
                disabled={busy}
                onPress={() => {
                  setShowReset(false);
                  setResetConfirmation("");
                }}
                style={styles.flexAction}
              />
              <Button
                label="Schedule reset"
                variant="danger"
                icon={<Trash2 size={16} color={tokens.danger} />}
                loading={busy}
                disabled={resetConfirmation !== "DELETE MY FORGE ANYWHERE DATA"}
                onPress={() => void scheduleReset()}
                style={styles.flexAction}
              />
            </View>
          </View>
        )}

        <Text style={[typeScale.monoMeta, styles.footnote, { color: tokens.ink4 }]}>
          Recover on a new device with GitHub sign-in plus your recovery phrase, or approval from a paired device.
        </Text>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 640, alignSelf: "center" },
  header: { gap: space.space4, marginBottom: space.space16 },
  headerTitle: { marginTop: space.space4 },
  row: {
    minHeight: 50,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: space.space8,
    paddingHorizontal: space.space12,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    marginTop: space.space8,
  },
  dangerRow: { borderWidth: 1 },
  rowLabel: { flex: 1 },
  resetPanel: { marginTop: space.space8, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 },
  actionRow: { flexDirection: "row", gap: space.space8 },
  flexAction: { flex: 1 },
  footnote: { marginTop: space.space20, lineHeight: 16, fontFamily: monoFamily.regular },
});
