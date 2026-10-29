// If the app has not reached its first interactive frame 15 s after launch, say so instead of
// leaving a spinner or a blank window: keep waiting, restart, or go and look for an update. Sits
// above every provider (it is exactly the stuck-provider case it exists for), so it only reads
// the module-level startup flag.
import * as SplashScreen from "expo-splash-screen";
import React, { useEffect, useState, useSyncExternalStore } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import { STARTUP_WATCHDOG_MS } from "../lib/crashCore";
import { isTauri } from "../lib/platform";
import { isStartupComplete, subscribeStartup } from "../lib/startupState";
import { ActionButton, UpdateActions, reloadApp, restartApp } from "./CrashScreen";

export function StartupWatchdog() {
  const complete = useSyncExternalStore(subscribeStartup, isStartupComplete, isStartupComplete);
  const [round, setRound] = useState(0);
  const [slow, setSlow] = useState(false);

  useEffect(() => {
    if (complete) return;
    const timer = setTimeout(() => {
      setSlow(true);
      // The native splash would otherwise sit on top of this prompt.
      SplashScreen.hideAsync().catch(() => undefined);
    }, STARTUP_WATCHDOG_MS);
    return () => clearTimeout(timer);
  }, [complete, round]);

  if (complete || !slow) return null;

  return (
    <View style={styles.overlay} accessibilityViewIsModal accessibilityRole="alert">
      <View style={styles.card}>
        <Text accessibilityRole="header" style={styles.title}>
          Forge is taking longer than expected to start
        </Text>
        <Text style={styles.detail}>It may still be connecting. You can wait, or try something else.</Text>
        <View style={styles.actions}>
          <ActionButton
            label="Keep waiting"
            primary
            onPress={() => {
              setSlow(false);
              setRound((value) => value + 1);
            }}
          />
          {Platform.OS === "web" ? <ActionButton label="Restart" onPress={() => void restartApp()} /> : null}
          {Platform.OS === "web" && !isTauri ? <ActionButton label="Reload" onPress={reloadApp} /> : null}
        </View>
        <UpdateActions />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  overlay: {
    position: "absolute",
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    backgroundColor: "rgba(17,17,17,0.94)",
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    zIndex: 1000,
  },
  card: { alignItems: "center", maxWidth: 480 },
  title: { color: "#eeeeee", fontSize: 18, fontWeight: "600", marginBottom: 8, textAlign: "center" },
  detail: { color: "#aaaaaa", fontSize: 14, marginBottom: 20, textAlign: "center" },
  actions: { flexDirection: "row", flexWrap: "wrap", justifyContent: "center", gap: 10 },
});
