// Settings (BUILD_PLAN §6 "Settings", Batch 1 W1). Reached from More. Shows the paired
// server, lets the user test the connection, re-pair, or forget the server, and surfaces
// the handful of app-level facts (reduce-motion status, version, theme) BUILD_PLAN calls
// for. Biometric app-lock is wired here (flips the pref AppLock.tsx gates on); native push
// is still N/A — the backend only speaks Web Push (see BUILD_PLAN §5 flag #1).
import { useQueryClient } from "@tanstack/react-query";
import Constants from "expo-constants";
import { router } from "expo-router";
import * as LocalAuthentication from "expo-local-authentication";
import React, { useCallback, useEffect, useState } from "react";
import { Alert, Platform, Switch, View } from "react-native";

import {
  Badge,
  ConfirmButton,
  ErrorText,
  ListRow,
  PrimaryButton,
  Screen,
  SectionTitle,
} from "../components/ui";
import { setBiometricLockEnabled, useBiometricLockEnabled } from "../components/AppLock";
import { type ConnectTestState, useAuth } from "../lib/auth";
import { useMotionEnabled } from "../lib/motion";
import { colors } from "../lib/theme";

const TEST_COPY: Record<Exclude<ConnectTestState, "idle" | "testing" | "ok">, string> = {
  "bad-token": "Pairing invalid — the token was rejected. Re-pair to fix this.",
  unreachable: "Server unreachable. Check the daemon is running and reachable.",
  "server-error": "The server returned an error. Check the `forge serve` logs.",
};

function maskToken(token: string | null): string {
  if (!token) return "—";
  return `…${token.slice(-4)}`;
}

function connectionKind(baseUrl: string | null): string {
  if (!baseUrl) return "—";
  if (baseUrl.startsWith("https:")) return "Remote (TLS)";
  if (baseUrl.startsWith("http:")) return "Local (plaintext)";
  return "—";
}

/** Groups rows into one bordered, rounded well — the sectioned-list look without doubling
 * Card's own padding on top of ListRow's (UI_RULES.md #30 rhythm consistency). */
function SectionWell({ children }: { children: React.ReactNode }) {
  return (
    <View className="bg-panel border border-borderSoft rounded-md overflow-hidden">
      {children}
    </View>
  );
}

export default function SettingsScreen() {
  const { host, token, baseUrl, forget, testConnection } = useAuth();
  const queryClient = useQueryClient();
  const motionEnabled = useMotionEnabled();

  const [testState, setTestState] = useState<ConnectTestState>("idle");
  const [forgetting, setForgetting] = useState(false);

  const biometricEnabled = useBiometricLockEnabled();
  const [biometricAvailable, setBiometricAvailable] = useState<boolean | null>(null);
  const [biometricUnavailableReason, setBiometricUnavailableReason] = useState<
    string | null
  >(null);
  const [biometricBusy, setBiometricBusy] = useState(false);

  useEffect(() => {
    if (Platform.OS === "web") {
      setBiometricAvailable(false);
      setBiometricUnavailableReason("Not available on web — use the app on a device.");
      return;
    }
    let cancelled = false;
    (async () => {
      const [hasHardware, isEnrolled] = await Promise.all([
        LocalAuthentication.hasHardwareAsync(),
        LocalAuthentication.isEnrolledAsync(),
      ]);
      if (cancelled) return;
      if (!hasHardware) {
        setBiometricAvailable(false);
        setBiometricUnavailableReason("This device has no Face ID/Touch ID hardware.");
      } else if (!isEnrolled) {
        setBiometricAvailable(false);
        setBiometricUnavailableReason("No Face ID/Touch ID enrolled — set it up in device Settings.");
      } else {
        setBiometricAvailable(true);
        setBiometricUnavailableReason(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const onToggleBiometric = useCallback(
    async (next: boolean) => {
      setBiometricBusy(true);
      try {
        await setBiometricLockEnabled(next);
      } finally {
        setBiometricBusy(false);
      }
    },
    [],
  );

  const onTest = useCallback(async () => {
    setTestState("testing");
    const result = await testConnection();
    setTestState(result);
  }, [testConnection]);

  const onRepair = useCallback(() => {
    Alert.alert(
      "Re-pair with a different server?",
      "Clears the current pairing so you can paste or scan a new connect URL.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Re-pair",
          onPress: async () => {
            await forget();
            queryClient.clear();
            router.replace("/connect");
          },
        },
      ],
    );
  }, [forget, queryClient]);

  const onForget = useCallback(() => {
    Alert.alert(
      "Forget this server?",
      "Removes the stored connect URL and clears cached data. You'll need to pair again to use Forge.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Forget",
          style: "destructive",
          onPress: async () => {
            setForgetting(true);
            await forget();
            queryClient.clear();
            setForgetting(false);
            router.replace("/connect");
          },
        },
      ],
    );
  }, [forget, queryClient]);

  return (
    <Screen contentContainerClassName="gap-16 pt-16">
      <View className="gap-6">
        <SectionTitle>Server</SectionTitle>
        <SectionWell>
          <ListRow title="Host" subtitle={host ?? "not paired"} subtitleEllipsize="head" />
          <ListRow title="Token" subtitle={maskToken(token)} />
          <ListRow title="Connection" subtitle={connectionKind(baseUrl)} />
        </SectionWell>
        <PrimaryButton
          label={testState === "testing" ? "Testing…" : "Test connection"}
          onPress={onTest}
          loading={testState === "testing"}
          fullWidth={false}
        />
        {testState === "ok" ? (
          <Badge label="Connected" tone="ok" />
        ) : testState !== "idle" && testState !== "testing" ? (
          <ErrorText message={TEST_COPY[testState]} onRetry={onTest} />
        ) : null}
      </View>

      <View className="gap-6">
        <SectionTitle>Pairing</SectionTitle>
        <PrimaryButton label="Re-pair with a different server" onPress={onRepair} />
        <ConfirmButton
          label="Forget this server"
          tone="no"
          onPress={onForget}
          loading={forgetting}
        />
      </View>

      <View className="gap-6">
        <SectionTitle>Privacy & alerts</SectionTitle>
        <SectionWell>
          <ListRow
            title="Biometric app lock"
            subtitle={
              biometricAvailable === false
                ? biometricUnavailableReason ?? "Unavailable on this device"
                : biometricEnabled
                  ? "On — Face ID/Touch ID required to open Forge"
                  : "Off — require Face ID/Touch ID to open Forge"
            }
            right={
              <Switch
                value={biometricAvailable === true && !!biometricEnabled}
                onValueChange={onToggleBiometric}
                disabled={biometricAvailable !== true || biometricBusy}
                trackColor={{ false: colors.border, true: colors.ok }}
                thumbColor={colors.dim}
              />
            }
          />
          <ListRow
            title="Push notifications"
            subtitle="Backend only supports Web Push — this app uses the live connection instead"
            right={<Badge label="N/A" tone="default" />}
          />
        </SectionWell>
      </View>

      <View className="gap-6">
        <SectionTitle>Accessibility</SectionTitle>
        <SectionWell>
          <ListRow
            title="Reduce motion"
            subtitle={
              motionEnabled
                ? "Off — animations play normally"
                : "On — animations are minimized"
            }
            right={
              <Switch
                value={!motionEnabled}
                disabled
                trackColor={{ false: colors.border, true: colors.accent }}
                thumbColor={colors.dim}
              />
            }
          />
        </SectionWell>
      </View>

      <View className="gap-6">
        <SectionTitle>About</SectionTitle>
        <SectionWell>
          <ListRow
            title="Forge"
            subtitle={`v${Constants.expoConfig?.version ?? "1.0.0"} · protocol 7`}
          />
          <ListRow title="Theme" subtitle="Dark only — matches the Forge control page" />
        </SectionWell>
      </View>
    </Screen>
  );
}
