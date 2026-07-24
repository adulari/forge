// Forge Anywhere — GitHub device-code sign-in (mobile.dc.html "AW Connect Signin"
// lines 464-471 — the device-code portion; the Direct/Anywhere choice cards above it
// belong to app/connect.tsx, outside this task's scope). Drives the real
// AnywhereProvider auth surface (the same startLogin/flow/error already used inline
// by index.tsx's SetupFlow) rather than the AnywhereClient mock's signInStart/
// signInPoll — this is real GitHub authentication, so it must not pretend to
// succeed against simulated data. index.tsx's own signed-out flow is untouched;
// this route is a restyled, dedicated entry point for the same real flow.
import { Redirect, router } from "expo-router";
import * as Clipboard from "expo-clipboard";
import { RefreshCw } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import Svg, { Path } from "react-native-svg";

import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";

const READY_ELSEWHERE_PHASES = new Set(["ready", "awaiting_approval", "new_recovery", "existing_recovery"]);

function GithubMark({ size, color }: { size: number; color: string }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.09.68-.22.68-.49v-1.7c-2.78.62-3.37-1.37-3.37-1.37-.45-1.18-1.11-1.5-1.11-1.5-.9-.63.07-.62.07-.62 1 .07 1.53 1.05 1.53 1.05.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.56-1.14-4.56-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.7 0 0 .84-.28 2.75 1.05a9.4 9.4 0 0 1 5 0c1.91-1.33 2.75-1.05 2.75-1.05.55 1.4.2 2.44.1 2.7.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.8-4.57 5.06.36.32.68.94.68 1.9v2.82c0 .27.18.59.69.49A10.06 10.06 0 0 0 22 12.25C22 6.58 17.52 2 12 2z" />
    </Svg>
  );
}

function useExpiryCountdown(deviceCode: string | undefined, expiresInSec: number | undefined): string | null {
  const [startedAt, setStartedAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  // Resetting the start time is a side effect (tied to a new device code), not a
  // render-time computation — done in an effect rather than useMemo(() => Date.now()).
  useEffect(() => {
    setStartedAt(Date.now());
  }, [deviceCode]);
  useEffect(() => {
    if (!deviceCode) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [deviceCode]);
  if (!deviceCode || expiresInSec == null) return null;
  const remaining = Math.max(0, expiresInSec - Math.round((now - startedAt) / 1000));
  const mm = Math.floor(remaining / 60);
  const ss = (remaining % 60).toString().padStart(2, "0");
  return `waiting… expires ${mm}m ${ss}s`;
}

export default function AnywhereSignInScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  // Called unconditionally (rules of hooks) even though its value is only shown
  // during the "authorizing" phase — `device_code` is undefined the rest of the time,
  // so the countdown itself resolves to null and simply isn't rendered.
  const expiryCountdown = useExpiryCountdown(anywhere.flow?.device_code, anywhere.flow?.expires_in);

  useEffect(() => {
    if (READY_ELSEWHERE_PHASES.has(anywhere.phase)) router.replace("/anywhere");
  }, [anywhere.phase]);

  if (READY_ELSEWHERE_PHASES.has(anywhere.phase)) return <Redirect href="/anywhere" />;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
        <Text accessibilityRole="header" style={[typeScale.title, styles.title, { color: tokens.ink }]}>
          Sign in with GitHub
        </Text>

        {anywhere.phase === "loading" || anywhere.phase === "starting" ? (
          <View style={styles.centerStep}>
            <ActivityIndicator color={tokens.accent} />
            <Text style={[typeScale.body, { color: tokens.ink2 }]}>Preparing secure sign-in…</Text>
          </View>
        ) : null}

        {anywhere.phase === "signed_out" ? (
          <View style={styles.step}>
            <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>
              GitHub identifies your Forge account. It never unlocks your encrypted sessions or replaces your
              recovery phrase.
            </Text>
            <Button
              label="Continue with GitHub"
              icon={<GithubMark size={18} color={tokens.onAccent} />}
              onPress={() => void anywhere.startLogin()}
              fullWidth
            />
          </View>
        ) : null}

        {anywhere.phase === "reauthentication_required" ? (
          <View style={styles.step}>
            <Text style={[typeScale.body, styles.measure, { color: tokens.ink2 }]}>
              This browser&apos;s secure session expired. Reconnect with GitHub, then approve this browser from an
              enrolled device.
            </Text>
            <Button
              label="Reconnect with GitHub"
              icon={<GithubMark size={18} color={tokens.onAccent} />}
              onPress={() => void anywhere.startLogin()}
              fullWidth
            />
          </View>
        ) : null}

        {anywhere.phase === "authorizing" ? (
          <View style={styles.step}>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Enter this code at github.com/login/device</Text>
            <View style={[styles.codeBox, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
              <Text
                selectable
                accessibilityLabel={`GitHub code ${anywhere.flow?.user_code ?? ""}`}
                style={[styles.deviceCode, tabularNums, { color: tokens.ink }]}
              >
                {anywhere.flow?.user_code ?? "••••-••••"}
              </Text>
            </View>
            <View style={styles.actionRow}>
              <Pressable
                onPress={() => {
                  if (anywhere.flow?.user_code) {
                    void Clipboard.setStringAsync(anywhere.flow.user_code);
                    toast.show("Code copied.", { tone: "neutral" });
                  }
                }}
                accessibilityRole="button"
                accessibilityLabel="Copy code"
                style={[styles.copyButton, { borderColor: tokens.border }]}
              >
                <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Copy code</Text>
              </Pressable>
              <View style={styles.waitingBox}>
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
                  {expiryCountdown ?? "waiting…"}
                </Text>
              </View>
            </View>
            <Button label="Open GitHub" variant="secondary" icon={<GithubMark size={18} color={tokens.ink} />} onPress={() => void anywhere.openLoginPage()} fullWidth />
            <Text style={[typeScale.monoMeta, styles.explainer, { color: tokens.ink4 }]}>
              One trial per GitHub account. Nothing is uploaded yet. New account → keys are generated on this device
              and a recovery phrase shown once. Returning → verify with your phrase or approve from a paired device.
            </Text>
          </View>
        ) : null}

        {anywhere.phase === "error" ? (
          <View style={styles.step}>
            <Text accessibilityRole="alert" style={[typeScale.body, styles.measure, { color: tokens.danger }]}>
              {anywhere.error ?? "That code expired or access was denied."}
            </Text>
            <View style={styles.chipRow}>
              <View style={[styles.chip, { borderColor: tokens.border }]}>
                <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Code expired · get a new one</Text>
              </View>
              <View style={[styles.chip, { borderColor: tokens.border }]}>
                <Text style={[typeScale.meta, { color: tokens.ink2 }]}>Access denied · start over</Text>
              </View>
            </View>
            <Button
              label="Start over"
              icon={<RefreshCw size={17} color={tokens.onAccent} />}
              onPress={anywhere.restartSetup}
              fullWidth
            />
          </View>
        ) : null}
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 640, alignSelf: "center" },
  title: { marginTop: space.space12 },
  measure: { maxWidth: 560 },
  centerStep: { minHeight: 160, alignItems: "center", justifyContent: "center", gap: space.space12, marginTop: space.space24 },
  step: { marginTop: space.space20, gap: space.space12 },
  codeBox: { minHeight: 68, borderWidth: 1, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" },
  deviceCode: { fontFamily: monoFamily.bold, fontSize: 22, lineHeight: 28, letterSpacing: 2.2 },
  actionRow: { flexDirection: "row", gap: space.space8 },
  copyButton: { flex: 1, minHeight: 44, borderWidth: 1, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center" },
  waitingBox: { flex: 1, minHeight: 44, alignItems: "center", justifyContent: "center" },
  explainer: { lineHeight: 17 },
  chipRow: { gap: space.space8 },
  chip: { borderWidth: 1, borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: space.space4, alignSelf: "flex-start" },
});
