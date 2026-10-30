import { Download } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";

interface InstallPromptEvent extends Event {
  prompt: () => Promise<void>;
}

function isStandalone(): boolean {
  if (typeof window === "undefined") return false;
  const nav = window.navigator as Navigator & { standalone?: boolean };
  return nav.standalone === true || window.matchMedia?.("(display-mode: standalone)").matches === true;
}

function isIosSafari(): boolean {
  if (typeof navigator === "undefined") return false;
  const ua = navigator.userAgent;
  return /iPhone|iPad|iPod/.test(ua) && /Safari/.test(ua) && !/CriOS|FxiOS|EdgiOS/.test(ua);
}

export function InstallHint() {
  const tokens = useTokens();
  const [prompt, setPrompt] = useState<InstallPromptEvent | null>(null);
  const [installed, setInstalled] = useState(false);
  const iosHint = isIosSafari() && !isStandalone();

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.addEventListener !== "function") return;
    const onPrompt = (event: Event) => {
      event.preventDefault();
      setPrompt(event as InstallPromptEvent);
    };
    const onInstalled = () => setInstalled(true);
    window.addEventListener("beforeinstallprompt", onPrompt);
    window.addEventListener("appinstalled", onInstalled);
    return () => {
      window.removeEventListener("beforeinstallprompt", onPrompt);
      window.removeEventListener("appinstalled", onInstalled);
    };
  }, []);

  if (installed || isStandalone()) return null;
  if (prompt) {
    return (
      <View style={styles.row}>
        <Button
          label="Install Forge"
          variant="secondary"
          icon={<Download size={16} strokeWidth={1.75} color={tokens.ink} />}
          onPress={() => {
            void prompt.prompt().finally(() => setPrompt(null));
          }}
          fullWidth
        />
      </View>
    );
  }
  if (iosHint) {
    return (
      <View style={styles.row}>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          to keep forge on your home screen, tap share, then add to home screen.
        </Text>
      </View>
    );
  }
  return null;
}

const styles = StyleSheet.create({
  row: { width: "100%", maxWidth: 480, gap: space.space8 },
});
