// Fallback surfaces for a dead or stuck app. They render above every provider, so they take no
// theme, query or auth context: plain primitives with fixed colours, like the boundary itself.
import * as Clipboard from "expo-clipboard";
import React, { useCallback, useEffect, useState } from "react";
import { Platform, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import {
  CRASH_COPY,
  NATIVE_CRASH_PARAM,
  buildCrashSummary,
  isReloadLoop,
  recentReloads,
  recordReload,
  type CrashInfo,
  type ReloadStorage,
} from "../lib/crashCore";
import { useAppVersion } from "../lib/appVersion";
import { isTauri } from "../lib/platform";
import { isStartupComplete } from "../lib/startupState";
import { checkDesktopUpdate, desktopUpdateAction, installDesktopUpdate, useDesktopUpdateState } from "../lib/updater";

function reloadStorage(): ReloadStorage | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

const canReload = Platform.OS === "web";

export function reloadApp(): void {
  if (!canReload || typeof window === "undefined") return;
  recordReload(reloadStorage(), Date.now());
  const url = new URL(window.location.href);
  if (url.searchParams.has(NATIVE_CRASH_PARAM)) {
    url.searchParams.delete(NATIVE_CRASH_PARAM);
    window.location.replace(url.toString());
  } else {
    window.location.reload();
  }
}

export async function restartApp(): Promise<void> {
  if (isTauri) {
    try {
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
      return;
    } catch {
      // Fall through: a reload is the next best thing to a restart.
    }
  }
  reloadApp();
}

export function ActionButton({
  label,
  onPress,
  primary,
  disabled,
}: {
  label: string;
  onPress: () => void;
  primary?: boolean;
  disabled?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      style={({ pressed }) => [
        styles.button,
        primary ? styles.buttonPrimary : null,
        pressed ? styles.buttonPressed : null,
        disabled ? styles.buttonDisabled : null,
      ]}
    >
      <Text style={[styles.buttonText, primary ? styles.buttonTextPrimary : null]}>{label}</Text>
    </Pressable>
  );
}

/** Check for updates / Install update, for the moments when the installed build is the problem. */
export function UpdateActions() {
  const update = useDesktopUpdateState();
  if (!isTauri) return null;
  const action = desktopUpdateAction(update);
  const busy = update.phase === "checking" || update.phase === "installing";
  const status =
    update.phase === "checking"
      ? "Checking for updates…"
      : update.phase === "up-to-date"
        ? "Forge is up to date."
        : update.phase === "available"
          ? `Version ${update.availableVersion ?? "?"} is available.`
          : update.phase === "installing"
            ? "Installing the update…"
            : update.phase === "error"
              ? "Couldn't reach the update server."
              : null;
  return (
    <View style={styles.updateBlock}>
      <View style={styles.actions}>
        <ActionButton label="Check for updates" onPress={() => void checkDesktopUpdate()} disabled={busy} />
        {action === "install" ? (
          <ActionButton label="Install update" onPress={() => void installDesktopUpdate()} disabled={busy} primary />
        ) : null}
      </View>
      {status ? (
        <Text accessibilityLiveRegion="polite" style={styles.status}>
          {status}
        </Text>
      ) : null}
    </View>
  );
}

export function CrashScreen({ info, onTryAgain }: { info: CrashInfo; onTryAgain?: () => void }) {
  const appVersion = useAppVersion();
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");
  const copy = CRASH_COPY[info.kind];
  const [looping] = useState(() => info.kind === "renderer-loop" || isReloadLoop(reloadStorage(), Date.now()));

  const copyDiagnostics = useCallback(() => {
    const summary = buildCrashSummary(info, {
      appVersion,
      platform: Platform.OS === "web" ? (isTauri ? "desktop" : "web") : Platform.OS,
      startupComplete: isStartupComplete(),
      reloadsRecently: recentReloads(reloadStorage(), Date.now()),
    });
    Clipboard.setStringAsync(summary)
      .then(() => setCopied("done"))
      .catch(() => setCopied("failed"));
  }, [appVersion, info]);

  useEffect(() => {
    if (copied === "idle") return;
    const timer = setTimeout(() => setCopied("idle"), 2500);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <View style={styles.container} accessibilityRole="alert">
      <ScrollView contentContainerStyle={styles.scroll}>
        <Text accessibilityRole="header" style={styles.title}>
          {copy.title}
        </Text>
        <Text style={styles.detail}>{copy.detail}</Text>
        <Text selectable style={styles.message} numberOfLines={6}>
          {info.message}
        </Text>
        {looping ? (
          <Text style={styles.warning}>
            Reloading hasn&apos;t fixed this. Copy the diagnostics and report it, or check for an update.
          </Text>
        ) : null}
        <View style={styles.actions}>
          {canReload ? <ActionButton label={looping ? "Reload anyway" : "Reload"} onPress={reloadApp} primary={!looping} /> : null}
          {onTryAgain && info.kind !== "renderer-loop" ? <ActionButton label="Try again" onPress={onTryAgain} /> : null}
          <ActionButton
            label={copied === "done" ? "Copied" : copied === "failed" ? "Couldn't copy" : "Copy diagnostics"}
            onPress={copyDiagnostics}
            primary={looping}
          />
        </View>
        <UpdateActions />
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: "#111111" },
  scroll: { flexGrow: 1, alignItems: "center", justifyContent: "center", paddingHorizontal: 24, paddingVertical: 32 },
  title: { color: "#eeeeee", fontSize: 18, fontWeight: "600", marginBottom: 8, textAlign: "center" },
  detail: { color: "#aaaaaa", fontSize: 14, marginBottom: 16, textAlign: "center", maxWidth: 440 },
  message: {
    color: "#999999",
    fontSize: 13,
    // Not monoFamily: the bundled mono font may not have loaded when this renders, and iOS has
    // no "monospace" alias.
    fontFamily: Platform.select({ ios: "Menlo", default: "monospace" }),
    textAlign: "center",
    marginBottom: 20,
    maxWidth: 520,
  },
  warning: { color: "#e0a040", fontSize: 13, marginBottom: 16, textAlign: "center", maxWidth: 440 },
  actions: { flexDirection: "row", flexWrap: "wrap", justifyContent: "center", gap: 10 },
  updateBlock: { marginTop: 16, alignItems: "center", gap: 8 },
  status: { color: "#999999", fontSize: 12 },
  button: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: "#555555",
    borderRadius: 8,
    paddingHorizontal: 18,
    paddingVertical: 10,
    justifyContent: "center",
  },
  buttonPrimary: { borderColor: "#ff8a3d", backgroundColor: "#2a1a10" },
  buttonPressed: { opacity: 0.7 },
  buttonDisabled: { opacity: 0.45 },
  buttonText: { color: "#dddddd", fontSize: 14, fontWeight: "500" },
  buttonTextPrimary: { color: "#ffb27a" },
});
