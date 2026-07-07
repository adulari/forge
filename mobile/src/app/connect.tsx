// Pairing screen (ARCHITECTURE.md §3 "the daemon contract", §7 App Store review
// posture; FEATURES.md §1.1/§4; DESIGN_SYSTEM.md §4 microcopy). Stands alone for
// an App Store reviewer with no daemon running — explains what Forge is with a
// bundled, no-network "how it works" note before any pairing UI. QR scan is the
// primary path (native camera / web "scan on your phone" hint via the
// `QRScan.native`/`.web` Metro split); a mono URL field covers paste + manual
// entry; `?url=` deep links prefill the field.
import { router, useLocalSearchParams } from "expo-router";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { QRScan } from "../components/pairing/QRScan";
import { haptics } from "../lib/haptics";
import { type ConnectTestState, parseConnectUrl, useAuth } from "../lib/auth";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

// DESIGN_SYSTEM §4 voice: lowercase-calm, says what happened + what to do.
// The unreachable copy carries the TLS guidance verbatim (FEATURES §3 /
// ARCHITECTURE §3) — this is not app-fixable, so it must always be shown in full.
const STATE_COPY: Record<Exclude<ConnectTestState, "idle" | "testing" | "ok">, string> = {
  "bad-token": "pairing invalid — re-scan the qr code or re-copy the connect url from `forge serve`.",
  unreachable:
    "server unreachable — is `forge serve` running? default `--lan` uses a self-signed certificate that native networking, browsers, and Tauri's WebView all reject. use `--anywhere` for a public tunnel with real TLS, or `--local` plus Tailscale/VPN to reach it directly.",
  "server-error": "the server returned an error — check the `forge serve` logs on the host.",
};

function decodeParam(raw: string): string {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

export default function ConnectScreen() {
  const tokens = useTokens();
  const { addServer, testConnection } = useAuth();
  const params = useLocalSearchParams<{ url?: string }>();
  const prefilled = useRef(false);

  const [url, setUrl] = useState("");
  const [testState, setTestState] = useState<ConnectTestState>("idle");
  const [formatError, setFormatError] = useState<string | null>(null);

  useEffect(() => {
    if (prefilled.current) return;
    const raw = params.url;
    if (typeof raw === "string" && raw.length > 0) {
      prefilled.current = true;
      setUrl(decodeParam(raw));
    }
  }, [params.url]);

  const busy = testState === "testing";

  const attemptConnect = useCallback(
    async (candidate: string) => {
      setFormatError(null);
      const parsed = parseConnectUrl(candidate);
      if (!parsed) {
        setTestState("idle");
        setFormatError(
          "that doesn't look like a forge connect url — paste it exactly as `forge serve` printed it, or scan its qr code.",
        );
        return;
      }
      setTestState("testing");
      const result = await testConnection(parsed.baseUrl);
      setTestState(result);
      if (result === "ok") {
        await addServer(candidate);
        haptics.pairSuccess();
        router.replace("/(tabs)");
      }
    },
    [addServer, testConnection],
  );

  const onScanned = useCallback(
    (data: string) => {
      setUrl(data);
      attemptConnect(data);
    },
    [attemptConnect],
  );

  const onConnectPress = () => attemptConnect(url);

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.hero}>
        <Text style={[type.display, styles.heroTitle, { color: tokens.ink }]}>Forge</Text>
        <Text style={[type.body, { color: tokens.ink2 }]}>
          Forge is a control surface for a fleet of AI coding agents. Connect to a{" "}
          <Text style={{ fontWeight: "600" }}>forge serve</Text> daemon on your machine or server
          to create sessions, review diffs, answer permission prompts, and keep tabs on every
          agent at once.
        </Text>
      </View>

      <Card style={styles.gapCard}>
        <SectionHeader>How it works</SectionHeader>
        <View style={styles.howItWorksBody}>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            1. run <Text style={{ fontWeight: "600" }}>forge serve</Text> on the machine where
            your code lives.
          </Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            2. scan the qr code it prints, or paste the connect url below.
          </Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            3. this app is just the window — your session state stays on that machine.
          </Text>
        </View>
      </Card>

      <Card variant="feature" style={styles.gapCard}>
        <SectionHeader>Scan to connect</SectionHeader>
        <QRScan onScanned={onScanned} paused={busy} />
      </Card>

      <Card style={styles.gapCard}>
        <Input
          label="Connect URL"
          mono
          value={url}
          onChangeText={(t) => {
            setUrl(t);
            setFormatError(null);
          }}
          placeholder="connect://host:7420/<token>"
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="go"
          editable={!busy}
          onSubmitEditing={onConnectPress}
          error={formatError ?? undefined}
        />
        <Button
          label={busy ? "Connecting…" : "Connect"}
          onPress={onConnectPress}
          loading={busy}
          disabled={busy || url.trim().length === 0}
          fullWidth
        />
      </Card>

      {testState !== "idle" && testState !== "testing" && testState !== "ok" ? (
        <Banner tone="danger" message={STATE_COPY[testState]} />
      ) : null}

      {testState === "ok" ? (
        <Text style={[type.body, styles.successText, { color: tokens.success }]}>
          connected — opening forge…
        </Text>
      ) : null}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  hero: { gap: space.space8 },
  heroTitle: { letterSpacing: -0.4 },
  gapCard: { gap: space.space12 },
  howItWorksBody: { gap: space.space8, paddingBottom: space.space4 },
  successText: { textAlign: "center" },
});
