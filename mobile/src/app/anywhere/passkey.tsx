import { KeyRound, ShieldCheck, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import { Screen } from "../../components/ds/Screen";
import { DEFAULT_ANYWHERE_SERVICE_URL } from "../../lib/anywhereApi";
import { completeBrowserPasskeySession } from "../../lib/anywherePasskeys";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

const SERVICE_URL = process.env.EXPO_PUBLIC_FORGE_ANYWHERE_URL ?? DEFAULT_ANYWHERE_SERVICE_URL;

export default function PasskeyBrowserScreen() {
  const tokens = useTokens();
  const [status, setStatus] = useState("Preparing secure recovery…");
  const [complete, setComplete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (Platform.OS !== "web" || typeof window === "undefined") {
      setError("Passkey recovery continues in your system browser.");
      return;
    }
    const parameters = new URLSearchParams(window.location.hash.replace(/^#/, ""));
    const session = parameters.get("session");
    window.history.replaceState(null, "", window.location.pathname);
    if (!session) {
      setError("This passkey recovery link is incomplete or expired.");
      return;
    }
    let active = true;
    void completeBrowserPasskeySession(SERVICE_URL, session, (next) => {
      if (active) setStatus(next);
    }).then((result) => {
      if (!active) return;
      setStatus(result === "registered" ? "Passkey registered" : "Recovery approved");
      setComplete(true);
    }).catch((reason) => {
      if (active) setError(reason instanceof Error ? reason.message : "Passkey recovery failed.");
    });
    return () => { active = false; };
  }, []);

  return (
    <Screen contentContainerStyle={styles.screen}>
      <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
        <View style={[styles.icon, { backgroundColor: error ? tokens.dangerBg : tokens.bg3 }]}>
          {error ? <X size={25} color={tokens.danger} /> : complete ? <ShieldCheck size={25} color={tokens.success} /> : <KeyRound size={25} color={tokens.accent} />}
        </View>
        <Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>Forge Anywhere recovery</Text>
        <Text accessibilityRole={error ? "alert" : undefined} style={[typeScale.body, styles.copy, { color: error ? tokens.danger : tokens.ink2 }]}>{error ?? status}</Text>
        <Text style={[typeScale.meta, styles.copy, { color: tokens.ink3 }]}>{complete ? "You can close this tab and return to Forge." : "Your recovery secret stays encrypted between this browser and your device. It is never added to this URL or sent to Forge."}</Text>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  screen: { minHeight: "100%", alignItems: "center", justifyContent: "center", padding: space.space20 },
  card: { width: "100%", maxWidth: 520, borderWidth: 1, borderRadius: radii.radius16, padding: space.space24, alignItems: "center", gap: space.space12 },
  icon: { width: 52, height: 52, borderRadius: 26, alignItems: "center", justifyContent: "center" },
  copy: { textAlign: "center", maxWidth: 430 },
});
