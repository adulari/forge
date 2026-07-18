// Forge Anywhere — GitHub device-code sign-in (mobile.dc.html "AW GitHub Sign-in",
// lines 155-210). Works signed OUT — this is the one Anywhere screen with no guard.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { Copy } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { goBackOr } from "../../lib/nav";
import { useAnywhere } from "../../lib/anywhere/store";
import type { DeviceCodeAuth } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";

const POLL_INTERVAL_MS = 2500;

function formatCountdown(totalSec: number): string {
  const clamped = Math.max(0, Math.round(totalSec));
  const m = Math.floor(clamped / 60);
  const s = clamped % 60;
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}

export default function AnywhereSignInScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { client, refresh } = useAnywhere();
  const { dotStyle } = useEmberdot("busy");

  const [auth, setAuth] = useState<DeviceCodeAuth | null>(null);
  const [secondsLeft, setSecondsLeft] = useState(0);
  const [networkError, setNetworkError] = useState(false);
  const pollTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopPolling = useCallback(() => {
    if (pollTimer.current != null) {
      clearInterval(pollTimer.current);
      pollTimer.current = null;
    }
  }, []);

  const poll = useCallback(async () => {
    try {
      const next = await client.signInPoll();
      setNetworkError(false);
      setAuth(next);
      if (next.state === "approved") {
        stopPolling();
        await refresh();
        // DeviceCodeAuth carries no isNewAccount flag (a real GitHub OAuth callback
        // would) — the mock backend seeds a fresh trial account with no existing
        // recovery phrase on every successful poll, so every completed sign-in here
        // is honestly treated as first-time setup rather than guessing at a signal
        // the client doesn't expose.
        router.replace("/anywhere/recovery-phrase");
      } else if (next.state === "expired" || next.state === "denied") {
        stopPolling();
      }
    } catch {
      setNetworkError(true);
      stopPolling();
    }
  }, [client, refresh, stopPolling]);

  const start = useCallback(async () => {
    setNetworkError(false);
    stopPolling();
    try {
      const next = await client.signInStart();
      setAuth(next);
      setSecondsLeft(next.expiresInSec);
      pollTimer.current = setInterval(() => void poll(), POLL_INTERVAL_MS);
    } catch {
      setNetworkError(true);
    }
  }, [client, poll, stopPolling]);

  useEffect(() => {
    void start();
    return stopPolling;
    // Mount-only: `start` is stable across the polling lifecycle it manages itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (auth?.state !== "waiting") return;
    const tick = setInterval(() => setSecondsLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(tick);
  }, [auth?.state]);

  const onCopy = useCallback(async () => {
    if (!auth) return;
    await Clipboard.setStringAsync(auth.code);
    toast.show("Code copied.");
  }, [auth, toast]);

  const waiting = auth?.state === "waiting" && !networkError;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <Pressable
          onPress={() => goBackOr("/anywhere")}
          accessibilityRole="button"
          accessibilityLabel="Back"
          hitSlop={8}
          style={styles.backHit}
        >
          <Text style={[type.bodyBold, { color: tokens.ink2 }]}>‹</Text>
        </Pressable>
        <Text style={[type.headingBold, { color: tokens.ink }]}>Sign in with GitHub</Text>
      </View>

      <View style={styles.codeBlock}>
        <Text style={[type.sub, styles.center, { color: tokens.ink2 }]}>Enter this code at</Text>
        <Text style={[type.code, styles.center, styles.verifyUrl, { color: tokens.ink }]}>
          {auth?.verifyUrl ?? "github.com/login/device"}
        </Text>
        <Text style={[styles.bigCode, tabularNums, { color: tokens.ink, fontFamily: monoFamily.bold }]}>
          {auth?.code ?? "········"}
        </Text>
        <Pressable
          onPress={onCopy}
          disabled={!auth}
          accessibilityRole="button"
          accessibilityLabel="Copy code"
          style={[styles.copyButton, { borderColor: tokens.borderStrong }]}
        >
          <Copy size={13} strokeWidth={2} color={tokens.ink2} />
          <Text style={[type.meta, { color: tokens.ink2 }]}>Copy code</Text>
        </Pressable>

        <View style={styles.waitingRow}>
          {waiting ? (
            <Animated.View style={[styles.pulseDot, dotStyle, { backgroundColor: tokens.accent }]} />
          ) : (
            <View style={[styles.pulseDot, { backgroundColor: networkError ? tokens.danger : tokens.ink4 }]} />
          )}
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            {networkError
              ? "Network failed while confirming."
              : auth?.state === "expired"
                ? "Code expired."
                : auth?.state === "denied"
                  ? "Access denied on GitHub."
                  : `Waiting for GitHub… code expires in ${formatCountdown(secondsLeft)}`}
          </Text>
        </View>
        <Text style={[type.meta, styles.footnote, { color: tokens.ink4 }]}>
          One trial per GitHub account. Nothing is uploaded yet.
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>if something goes wrong</Text>
        <TroubleRow
          label="Code expired"
          actionLabel="Get a new code"
          onPress={() => void start()}
        />
        <TroubleRow label="Access denied on GitHub" actionLabel="Start over" onPress={() => void start()} />
        <TroubleRow
          label="Network failed while confirming"
          actionLabel="Retry"
          dotColor={tokens.danger}
          onPress={() => void poll()}
          showSeparator={false}
        />
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>next</Text>
        <Text style={[type.sub, styles.nextCopy, { color: tokens.ink3 }]}>
          New account → we generate encryption keys on this device and show a recovery phrase
          once. Returning account → verify with your recovery phrase or approve from a paired
          device.
        </Text>
      </View>

      <Pressable
        onPress={() => router.replace("/anywhere")}
        accessibilityRole="button"
        accessibilityLabel="Cancel"
        style={styles.cancel}
      >
        <Text style={[type.bodyBold, { color: tokens.ink3 }]}>Cancel</Text>
      </Pressable>
    </Screen>
  );
}

function TroubleRow({
  label,
  actionLabel,
  onPress,
  dotColor,
  showSeparator = true,
}: {
  label: string;
  actionLabel: string;
  onPress: () => void;
  dotColor?: string;
  showSeparator?: boolean;
}) {
  const tokens = useTokens();
  return (
    <View>
      <View style={styles.troubleRow}>
        {dotColor ? <View style={[styles.smallDot, { backgroundColor: dotColor }]} /> : null}
        <Text style={[type.sub, styles.troubleLabel, { color: tokens.ink2 }]}>{label}</Text>
        <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={actionLabel} hitSlop={8}>
          <Text style={[type.meta, { color: tokens.accent }]}>{actionLabel}</Text>
        </Pressable>
      </View>
      {showSeparator ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget },
  backHit: { width: tapTarget, height: tapTarget, alignItems: "center", justifyContent: "center", marginLeft: -space.space8 },
  codeBlock: { marginTop: space.space32, alignItems: "center" },
  center: { textAlign: "center" },
  verifyUrl: { marginTop: 3 },
  bigCode: { fontSize: 34, letterSpacing: 4, marginTop: space.space24 },
  copyButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
    marginTop: space.space12,
    height: 36,
    paddingHorizontal: space.space16,
    borderRadius: radii.radius8,
    borderWidth: 1,
  },
  waitingRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space24 },
  pulseDot: { width: 8, height: 8, borderRadius: 4 },
  footnote: { marginTop: space.space8, textAlign: "center" },
  section: { marginTop: space.space32 },
  nextCopy: { marginTop: space.space8, lineHeight: 19 },
  troubleRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 },
  troubleLabel: { flex: 1 },
  smallDot: { width: 7, height: 7, borderRadius: 3.5 },
  hairline: { height: StyleSheet.hairlineWidth },
  cancel: { alignItems: "center", justifyContent: "center", minHeight: tapTarget, marginTop: space.space24 },
});
