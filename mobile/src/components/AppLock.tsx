// Biometric app-lock gate (T2.1 — "port the existing Face ID gate onto ds primitives").
// Wraps the whole app in root `_layout.tsx`: when the user has enabled "Require Face ID" in
// Settings, this blocks content behind a lock screen on cold start and every foreground
// return, until `LocalAuthentication.authenticateAsync` succeeds. When the pref is off
// (default), this is a no-op passthrough. No-op on web (no native biometric API there).
//
// The pref lives in AsyncStorage key `forge.appLock` ("true"/"false") — see the HANDOFF
// comment in `src/app/(tabs)/settings.tsx` (T2.2, owns the toggle UI). That screen writes
// the key directly rather than through a shared setter here, since the two tasks' file
// scopes are disjoint; this gate re-reads the key fresh on every foreground transition (not
// just once at cold start), so a change made in Settings takes effect on the very next
// background/foreground cycle without needing cross-file pub/sub.
import AsyncStorage from "@react-native-async-storage/async-storage";
import * as LocalAuthentication from "expo-local-authentication";
import type { LocalAuthenticationError } from "expo-local-authentication";
import { Lock } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { ActivityIndicator, AppState, Platform, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Button } from "./ds/Button";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

const APP_LOCK_KEY = "forge.appLock";

async function readAppLockEnabled(): Promise<boolean> {
  const raw = await AsyncStorage.getItem(APP_LOCK_KEY);
  return raw === "true";
}

const AUTH_ERROR_COPY: Partial<Record<LocalAuthenticationError, string>> = {
  user_cancel: "cancelled — tap unlock to try again.",
  system_cancel: "cancelled — tap unlock to try again.",
  user_fallback: "tap unlock to try again.",
  lockout: "too many attempts — unlock your device, then reopen Forge.",
  not_enrolled: "no Face ID/Touch ID enrolled on this device.",
  not_available: "biometric authentication isn't available on this device.",
  passcode_not_set: "set a device passcode to use app lock.",
};

type LockPhase = "checking" | "locked" | "unlocked";

export function AppLock({ children }: { children: React.ReactNode }) {
  const tokens = useTokens();
  const [phase, setPhase] = useState<LockPhase>("checking");
  const [authError, setAuthError] = useState<string | null>(null);
  const [authenticating, setAuthenticating] = useState(false);
  const appStateRef = useRef(AppState.currentState);
  // Last-read value of the pref, kept in sync by `evaluateLock` so the AppState listener can
  // decide synchronously whether to re-cover the screen the instant the app backgrounds
  // (rather than waiting on an AsyncStorage read while the app switcher is snapshotting it).
  const enabledRef = useRef(false);

  const tryAuthenticate = useCallback(async () => {
    setAuthenticating(true);
    setAuthError(null);
    try {
      const result = await LocalAuthentication.authenticateAsync({
        promptMessage: "Unlock Forge",
      });
      if (result.success) {
        setPhase("unlocked");
      } else {
        setPhase("locked");
        setAuthError(AUTH_ERROR_COPY[result.error] ?? "authentication failed — try again.");
      }
    } finally {
      setAuthenticating(false);
    }
  }, []);

  // Re-reads the persisted pref, then either passes through, or verifies hardware is still
  // usable (fail-open if not, rather than locking the user out forever), then locks + prompts.
  const evaluateLock = useCallback(async () => {
    if (Platform.OS === "web") {
      enabledRef.current = false;
      setPhase("unlocked");
      return;
    }
    const enabled = await readAppLockEnabled();
    enabledRef.current = enabled;
    if (!enabled) {
      setPhase("unlocked");
      return;
    }
    const [hasHardware, isEnrolled] = await Promise.all([
      LocalAuthentication.hasHardwareAsync(),
      LocalAuthentication.isEnrolledAsync(),
    ]);
    if (!hasHardware || !isEnrolled) {
      setPhase("unlocked");
      return;
    }
    setPhase("locked");
    tryAuthenticate();
  }, [tryAuthenticate]);

  // Cold start.
  useEffect(() => {
    evaluateLock();
  }, [evaluateLock]);

  // Re-lock the instant the app backgrounds (using the last-known pref, synchronously);
  // re-evaluate (fresh pref read) + re-authenticate on every return to foreground.
  useEffect(() => {
    const sub = AppState.addEventListener("change", (next) => {
      const prev = appStateRef.current;
      appStateRef.current = next;
      if (prev === "active" && next.match(/inactive|background/) && enabledRef.current) {
        setPhase("locked");
      } else if (prev.match(/inactive|background/) && next === "active") {
        evaluateLock();
      }
    });
    return () => sub.remove();
  }, [evaluateLock]);

  if (phase === "checking") {
    return (
      <View style={[styles.flex, styles.center, { backgroundColor: tokens.bg1 }]}>
        <ActivityIndicator color={tokens.ink3} />
      </View>
    );
  }

  return (
    <View style={styles.flex}>
      {children}
      {phase === "locked" ? (
        <SafeAreaView style={[styles.overlay, styles.center, { backgroundColor: tokens.bg1 }]}>
          <Lock size={32} color={tokens.accent} strokeWidth={1.75} />
          <Text style={[type.heading, styles.title, { color: tokens.ink }]}>Forge is locked</Text>
          <Text style={[type.sub, styles.message, { color: tokens.ink2 }]}>
            unlock with Face ID to continue.
          </Text>
          {authError ? (
            <Text style={[type.sub, styles.message, { color: tokens.danger }]}>{authError}</Text>
          ) : null}
          <Button
            label={authenticating ? "Unlocking…" : "Unlock"}
            onPress={tryAuthenticate}
            loading={authenticating}
            style={styles.unlockButton}
          />
        </SafeAreaView>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  center: { alignItems: "center", justifyContent: "center" },
  overlay: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 50,
    elevation: 50,
    paddingHorizontal: space.space24,
  },
  title: { marginTop: space.space16 },
  message: { textAlign: "center", marginTop: space.space4 },
  unlockButton: { marginTop: space.space20 },
});
