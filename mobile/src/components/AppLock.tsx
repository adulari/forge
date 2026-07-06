// Biometric app-lock gate (BUILD_PLAN §6/§10 native-deps batch). Wraps the whole app in
// root `_layout.tsx`: when the user has enabled "Require Face ID" in Settings, this blocks
// content behind a lock screen on cold start and every foreground return, until
// `LocalAuthentication.authenticateAsync` succeeds. When the pref is off (default), this is
// a no-op passthrough.
//
// The pref lives in AsyncStorage (not expo-secure-store — it's a UI preference, not a
// credential) behind a tiny module-level cache + pub/sub so Settings' toggle and this gate
// agree instantly within the same session, without waiting for a relaunch.
import AsyncStorage from "@react-native-async-storage/async-storage";
import * as LocalAuthentication from "expo-local-authentication";
import type { LocalAuthenticationError } from "expo-local-authentication";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { ActivityIndicator, AppState, Platform, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { PrimaryButton } from "./ui";
import { theme } from "../lib/theme";

const BIOMETRIC_LOCK_KEY = "forge.biometricLockEnabled";

let cachedEnabled: boolean | null = null;
const listeners = new Set<(enabled: boolean) => void>();

async function loadBiometricLockEnabled(): Promise<boolean> {
  if (cachedEnabled != null) return cachedEnabled;
  const raw = await AsyncStorage.getItem(BIOMETRIC_LOCK_KEY);
  cachedEnabled = raw === "true";
  return cachedEnabled;
}

/** Flips the pref, persists it, and notifies every `useBiometricLockEnabled()` subscriber
 * (Settings' toggle and this gate's effect) immediately — no relaunch needed to take effect. */
export async function setBiometricLockEnabled(enabled: boolean): Promise<void> {
  cachedEnabled = enabled;
  await AsyncStorage.setItem(BIOMETRIC_LOCK_KEY, enabled ? "true" : "false");
  listeners.forEach((listener) => listener(enabled));
}

/** `null` while the initial AsyncStorage read is in flight — callers that gate content on
 * this (AppLock) must treat `null` as "unknown, keep blocking" to avoid a cold-start flash. */
export function useBiometricLockEnabled(): boolean | null {
  const [enabled, setEnabled] = useState<boolean | null>(cachedEnabled);

  useEffect(() => {
    let mounted = true;
    loadBiometricLockEnabled().then((value) => {
      if (mounted) setEnabled(value);
    });
    const listener = (value: boolean) => setEnabled(value);
    listeners.add(listener);
    return () => {
      mounted = false;
      listeners.delete(listener);
    };
  }, []);

  return enabled;
}

const AUTH_ERROR_COPY: Partial<Record<LocalAuthenticationError, string>> = {
  user_cancel: "Cancelled — tap Unlock to try again.",
  system_cancel: "Cancelled — tap Unlock to try again.",
  user_fallback: "Tap Unlock to try again.",
  lockout: "Too many attempts — unlock your device, then reopen Forge.",
  not_enrolled: "No Face ID/Touch ID enrolled on this device.",
  not_available: "Biometric authentication isn't available on this device.",
  passcode_not_set: "Set a device passcode to use app lock.",
};

type LockPhase = "checking" | "locked" | "unlocked";

export function AppLock({ children }: { children: React.ReactNode }) {
  const enabled = useBiometricLockEnabled();
  const [phase, setPhase] = useState<LockPhase>("checking");
  const [authError, setAuthError] = useState<string | null>(null);
  const [authenticating, setAuthenticating] = useState(false);
  const appStateRef = useRef(AppState.currentState);

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
        setAuthError(AUTH_ERROR_COPY[result.error] ?? "Authentication failed. Try again.");
      }
    } finally {
      setAuthenticating(false);
    }
  }, []);

  // Resolve the initial phase whenever the pref (re)loads: unknown -> keep checking (no
  // flash), off -> passthrough, on -> verify hardware is still usable (fail-open if not,
  // rather than locking the user out forever) then lock + auth immediately (cold start).
  useEffect(() => {
    if (enabled === null) {
      setPhase("checking");
      return;
    }
    if (!enabled || Platform.OS === "web") {
      setPhase("unlocked");
      return;
    }
    let cancelled = false;
    (async () => {
      const [hasHardware, isEnrolled] = await Promise.all([
        LocalAuthentication.hasHardwareAsync(),
        LocalAuthentication.isEnrolledAsync(),
      ]);
      if (cancelled) return;
      if (!hasHardware || !isEnrolled) {
        setPhase("unlocked");
        return;
      }
      setPhase("locked");
      tryAuthenticate();
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled]);

  // Re-lock on backgrounding, re-authenticate on every return to foreground.
  useEffect(() => {
    if (!enabled || Platform.OS === "web") return;
    const sub = AppState.addEventListener("change", (next) => {
      const prev = appStateRef.current;
      appStateRef.current = next;
      if (prev === "active" && next.match(/inactive|background/)) {
        setPhase("locked");
      } else if (prev.match(/inactive|background/) && next === "active") {
        tryAuthenticate();
      }
    });
    return () => sub.remove();
  }, [enabled, tryAuthenticate]);

  if (phase === "checking") {
    return (
      <SafeAreaView className="flex-1 bg-bg items-center justify-center">
        <ActivityIndicator color={theme.colors.dim} />
      </SafeAreaView>
    );
  }

  return (
    <View style={{ flex: 1 }}>
      {children}
      {phase === "locked" ? (
        <View
          className="absolute inset-0 bg-bg items-center justify-center gap-16 px-16"
          style={{ zIndex: 50, elevation: 50 }}
        >
          <Text className="text-accent text-[16px] font-bold">⚒ Forge is locked</Text>
          <Text className="text-dim text-[13px] text-center">
            Unlock with Face ID to continue.
          </Text>
          {authError ? (
            <Text className="text-no text-[13px] text-center">{authError}</Text>
          ) : null}
          <PrimaryButton
            label={authenticating ? "Unlocking…" : "Unlock"}
            onPress={tryAuthenticate}
            loading={authenticating}
            fullWidth={false}
          />
        </View>
      ) : null}
    </View>
  );
}
