// Pairing screen (ARCHITECTURE.md §3 "the daemon contract", §7 App Store review
// posture; FEATURES.md §1.1/§4; DESIGN_SYSTEM.md §4 microcopy). Stands alone for
// an App Store reviewer with no daemon running — explains what Forge is with a
// bundled, no-network "how it works" note before any pairing UI. QR scan is the
// primary path (native camera / web "scan on your phone" hint via the
// `QRScan.native`/`.web` Metro split); a mono URL field covers paste + manual
// entry; `?url=` deep links prefill the field.
import { router, useLocalSearchParams } from "expo-router";
import { X } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { Card } from "../components/ds/Card";
import { IconButton } from "../components/ds/IconButton";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { StatusDot } from "../components/ds/StatusDot";
import { useToast } from "../components/ds/ToastHost";
import { QRScan } from "../components/pairing/QRScan";
import { type ConnectTestState, parseConnectUrl, useAuth } from "../lib/auth";
import {
  detectForgeServe,
  forgeBinaryAvailable,
  pollForForgeServe,
  startForgeServe,
  type DetectedServeState,
} from "../lib/desktopServe";
import { haptics } from "../lib/haptics";
import { goBackOr } from "../lib/nav";
import { isTauri } from "../lib/platform";
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

// Desktop auto-detect (Tauri only, first-run only — ARCHITECTURE.md §6.4). "idle"/"detecting"/
// "unavailable" render nothing so the screen never flashes a card that's about to disappear;
// everything else augments the manual/QR flow below, never replaces it.
type DesktopAutoState =
  | { kind: "idle" }
  | { kind: "detecting" }
  | { kind: "found"; state: DetectedServeState }
  | { kind: "found-lan"; state: DetectedServeState }
  | { kind: "offer-start" }
  | { kind: "starting" }
  | { kind: "start-failed"; message: string }
  | { kind: "unavailable" };

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export default function ConnectScreen() {
  const tokens = useTokens();
  const { addServer, testConnection, servers } = useAuth();
  const toast = useToast();
  const params = useLocalSearchParams<{ url?: string }>();
  // Tracks the last `?url=` value we already applied (not just "did we ever apply one") so a
  // second deep link while this screen is still mounted re-prefills instead of being dropped.
  const lastAppliedUrl = useRef<string | null>(null);
  const attemptRef = useRef(0);
  // Reached either as the first-run pairing screen (no back stack) or pushed from Settings'
  // "Add server" (has a back stack) — the latter gets a close affordance and must not steal
  // the active connection out from under the user (see attemptConnect below).
  const canClose = router.canGoBack();

  const [url, setUrl] = useState("");
  const [scanEnabled, setScanEnabled] = useState(false);
  const [testState, setTestState] = useState<ConnectTestState>("idle");
  const [formatError, setFormatError] = useState<string | null>(null);

  useEffect(() => {
    const raw = params.url;
    if (typeof raw !== "string" || raw.length === 0) return;
    if (lastAppliedUrl.current === raw) return;
    lastAppliedUrl.current = raw;
    setUrl(decodeParam(raw));
    // A fresh deep link supersedes whatever the previous attempt on this screen left behind.
    setTestState("idle");
    setFormatError(null);
  }, [params.url]);

  const busy = testState === "testing";

  const onClose = useCallback(() => goBackOr("/(tabs)/settings"), []);

  const attemptConnect = useCallback(
    async (candidate: string) => {
      const attempt = ++attemptRef.current;
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
      let result: ConnectTestState;
      try {
        result = await testConnection(parsed.baseUrl);
      } catch {
        result = "unreachable";
      }
      if (attempt !== attemptRef.current) return;
      setTestState(result);
      if (result === "ok") {
        // Pushed from Settings ("Add server") — add it alongside the existing servers without
        // switching the active connection out from under the user; first-run (no back stack)
        // keeps the original behavior of activating the only server it just added.
        const additional = canClose;
        try {
          const added = await addServer(candidate, { setActive: !additional });
          if (attempt !== attemptRef.current) return;
          haptics.pairSuccess();
          if (additional) {
            toast.show(`added ${added.name}`, { tone: "success" });
            goBackOr("/(tabs)/settings");
          } else {
            router.replace("/(tabs)");
          }
        } catch {
          if (attempt !== attemptRef.current) return;
          setTestState("server-error");
        }
      }
    },
    [addServer, testConnection, canClose, toast],
  );

  // First-run only (no servers stored yet) and Tauri only — detect a locally running
  // `forge serve` (or offer to start one) instead of making the user paste a URL.
  const [desktopState, setDesktopState] = useState<DesktopAutoState>({ kind: "idle" });
  const noServersYet = servers.length === 0;

  useEffect(() => {
    if (!isTauri || !noServersYet) return;
    let cancelled = false;
    setDesktopState({ kind: "detecting" });
    void (async () => {
      const found = await detectForgeServe();
      if (cancelled) return;
      if (found) {
        setDesktopState(
          found.exposure === "lan" ? { kind: "found-lan", state: found } : { kind: "found", state: found },
        );
        return;
      }
      const available = await forgeBinaryAvailable();
      if (cancelled) return;
      setDesktopState(available ? { kind: "offer-start" } : { kind: "unavailable" });
    })();
    return () => {
      cancelled = true;
    };
  }, [noServersYet]);

  const onFoundConnectPress = useCallback(() => {
    if (desktopState.kind !== "found") return;
    // Hand off to the same test/add/navigate flow the manual field and QR scan use — the card's
    // own job ends here, the status banner below takes over.
    setDesktopState({ kind: "idle" });
    void attemptConnect(desktopState.state.base_url);
  }, [desktopState, attemptConnect]);

  const onStartServerPress = useCallback(async () => {
    setDesktopState({ kind: "starting" });
    try {
      await startForgeServe();
    } catch (err) {
      setDesktopState({ kind: "start-failed", message: errorMessage(err) });
      return;
    }
    const found = await pollForForgeServe();
    if (!found) {
      setDesktopState({
        kind: "start-failed",
        message:
          "forge serve didn't come up within 15s — check the terminal it prints to, or paste the connect url manually below.",
      });
      return;
    }
    setDesktopState({ kind: "idle" });
    void attemptConnect(found.base_url);
  }, [attemptConnect]);

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
      {/* Only the pushed "Add server" flow (Settings) has anywhere to go back to — first-run
          pairing has no back stack, and Tauri has no browser chrome to fall back on either. */}
      {canClose ? (
        <View style={styles.closeRow}>
          <IconButton
            icon={<X size={20} strokeWidth={1.75} color={tokens.ink} />}
            onPress={onClose}
            accessibilityLabel="Close"
          />
        </View>
      ) : null}
      <View style={styles.hero}>
        <Text style={[type.display, styles.heroTitle, { color: tokens.ink }]}>Forge</Text>
        <Text style={[type.body, { color: tokens.ink2 }]}>
          Forge is a control surface for a fleet of AI coding agents. Connect to a{" "}
          <Text style={{ fontWeight: "600" }}>forge serve</Text> daemon on your machine or server
          to create sessions, review diffs, answer permission prompts, and keep tabs on every
          agent at once.
        </Text>
      </View>

      {desktopState.kind === "found" ? (
        <Card variant="feature" style={styles.gapCard}>
          <SectionHeader>This machine</SectionHeader>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            found forge running on this machine, port {desktopState.state.port}.
          </Text>
          <Button label="Connect" onPress={onFoundConnectPress} loading={busy} disabled={busy} fullWidth />
        </Card>
      ) : null}

      {desktopState.kind === "found-lan" ? (
        <Card variant="feature" style={styles.gapCard}>
          <SectionHeader>This machine</SectionHeader>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            forge is running on this machine over <Text style={{ fontWeight: "600" }}>--lan</Text>, but its
            self-signed certificate isn&apos;t trusted here — restart it with{" "}
            <Text style={{ fontWeight: "600" }}>forge serve --local</Text> or{" "}
            <Text style={{ fontWeight: "600" }}>forge serve --anywhere</Text> to connect from this app.
          </Text>
        </Card>
      ) : null}

      {desktopState.kind === "offer-start" ? (
        <Card variant="feature" style={styles.gapCard}>
          <SectionHeader>This machine</SectionHeader>
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            forge is installed on this machine — start a local server?
          </Text>
          <Button label="Start server" onPress={() => void onStartServerPress()} fullWidth />
        </Card>
      ) : null}

      {desktopState.kind === "starting" ? (
        <Card variant="feature" style={styles.gapCard}>
          <SectionHeader>This machine</SectionHeader>
          <View style={styles.startingRow}>
            <StatusDot state="busy" />
            <Text style={[type.sub, { color: tokens.ink2 }]}>starting your local forge server…</Text>
          </View>
        </Card>
      ) : null}

      {desktopState.kind === "start-failed" ? (
        <Card variant="feature" style={styles.gapCard}>
          <SectionHeader>This machine</SectionHeader>
          <Text style={[type.sub, { color: tokens.danger }]}>{desktopState.message}</Text>
          <Button
            label="Try again"
            variant="secondary"
            onPress={() => void onStartServerPress()}
            fullWidth
          />
        </Card>
      ) : null}

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
        <QRScan onScanned={onScanned} enabled={scanEnabled} paused={busy} />
        <Button label={scanEnabled ? "Stop scanning" : "Scan QR code"} variant="secondary" onPress={() => setScanEnabled((enabled) => !enabled)} fullWidth />
      </Card>

      <Card style={styles.gapCard}>
        <Input
          label="Connect URL"
          mono
          value={url}
          onChangeText={(t) => {
            setUrl(t.trimStart());
            setFormatError(null);
            setTestState("idle");
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

      <Card variant="feature" style={styles.gapCard}>
        <Text style={[type.heading, { color: tokens.ink }]}>Connect from anywhere</Text>
        <Text style={[type.sub, { color: tokens.ink2 }]}>Forge Anywhere adds managed, end-to-end encrypted access and sync. Direct, LAN, and your own tunnels remain free.</Text>
        <Button label="Set up Forge Anywhere" variant="secondary" onPress={() => router.push("/anywhere")} fullWidth />
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
  closeRow: { flexDirection: "row", justifyContent: "flex-end" },
  hero: { gap: space.space8 },
  heroTitle: { letterSpacing: -0.4 },
  gapCard: { gap: space.space12 },
  howItWorksBody: { gap: space.space8, paddingBottom: space.space4 },
  startingRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  successText: { textAlign: "center" },
});
