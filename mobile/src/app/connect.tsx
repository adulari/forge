// Pairing screen (ARCHITECTURE.md §3 "the daemon contract", §7 App Store review
// posture; FEATURES.md §1.1/§4). Hearth (mobile.dc.html:1181, desktop.dc.html:1105,
// web.dc.html:430): hero + numbered steps, no cards — the QR reticle and the mono
// connect-URL field are the only bordered boxes on this screen. Stands alone for an
// App Store reviewer with no daemon running — explains what Forge is with a bundled,
// no-network "how it works" note before any pairing UI. QR scan is the primary path
// (native camera / web "scan on your phone" hint via the `QRScan.native`/`.web` Metro
// split); a mono URL field covers paste + manual entry; `?url=` deep links prefill it.
import { router, useLocalSearchParams } from "expo-router";
import { Flame, X } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { IconButton } from "../components/ds/IconButton";
import { Input } from "../components/ds/Input";
import { Screen } from "../components/ds/Screen";
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
import { type as typeScale } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

// DESIGN_SYSTEM §4 voice: lowercase-calm, says what happened + what to do.
// The unreachable copy carries the TLS guidance verbatim (FEATURES §3 /
// ARCHITECTURE §3) — this is not app-fixable, so it must always be shown in full.
const STATE_COPY: Record<Exclude<ConnectTestState, "idle" | "testing" | "ok">, string> = {
  "bad-token": "pairing invalid — re-scan the qr code or re-copy the connect url from `forge serve`.",
  unreachable:
    "server unreachable — is `forge serve` running? default `--lan` uses a self-signed certificate that native networking, browsers, and Tauri's WebView all reject. use `--anywhere` for a public tunnel with real TLS, or `--local` plus Tailscale/VPN to reach it directly.",
  "server-error": "the server returned an error — check the `forge serve` logs on the host.",
};

const STEPS: { key: string; text: string }[] = [
  { key: "1", text: "run forge serve where your code lives" },
  { key: "2", text: "scan the qr code it prints, or paste the connect url" },
  { key: "3", text: "answer, review and forge from anywhere" },
];

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
  const { isCompact } = useBreakpoint();
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
    <Screen
      scroll
      keyboardAvoiding
      contentContainerStyle={isCompact ? styles.content : { ...styles.content, ...styles.contentWide }}
    >
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
        <View style={styles.heroTitleRow}>
          <Flame size={isCompact ? 26 : 28} strokeWidth={1.75} color={tokens.accent} />
          <Text style={[typeScale.display, { color: tokens.ink }]}>Forge</Text>
        </View>
        <Text style={[typeScale.body, { color: tokens.ink2 }]}>
          A control surface for your fleet of coding agents. Connect to a{" "}
          <Text style={[typeScale.codeSmall, { color: tokens.ink }]}>forge serve</Text> daemon —
          this app is just the window; your sessions stay on that machine.
        </Text>
      </View>

      <View style={styles.steps}>
        {STEPS.map((step) => (
          <View key={step.key} style={styles.stepRow}>
            <Text style={[typeScale.monoMeta, styles.stepIndex, { color: tokens.accent }]}>{step.key}</Text>
            <Text style={[typeScale.sub, styles.stepText, { color: tokens.ink2 }]}>{step.text}</Text>
          </View>
        ))}
      </View>

      {desktopState.kind === "found" ? (
        <View style={styles.autoRow}>
          <StatusDot state="idle" />
          <Text style={[typeScale.monoMeta, styles.autoText, { color: tokens.ink4 }]}>
            found on this machine: forge serve, port {desktopState.state.port} ·{" "}
            <Text onPress={onFoundConnectPress} style={{ color: tokens.accent }}>
              connect
            </Text>
          </Text>
        </View>
      ) : null}

      {desktopState.kind === "found-lan" ? (
        <Text style={[typeScale.sub, styles.autoHint, { color: tokens.ink2 }]}>
          forge is running on this machine over <Text style={{ fontWeight: "600" }}>--lan</Text>, but its
          self-signed certificate isn&apos;t trusted here — restart it with{" "}
          <Text style={{ fontWeight: "600" }}>forge serve --local</Text> or{" "}
          <Text style={{ fontWeight: "600" }}>forge serve --anywhere</Text> to connect from this app.
        </Text>
      ) : null}

      {desktopState.kind === "offer-start" ? (
        <View style={styles.autoBlock}>
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>forge is installed on this machine — start a local server?</Text>
          <Button label="Start server" onPress={() => void onStartServerPress()} />
        </View>
      ) : null}

      {desktopState.kind === "starting" ? (
        <View style={styles.autoRow}>
          <StatusDot state="busy" />
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>starting your local forge server…</Text>
        </View>
      ) : null}

      {desktopState.kind === "start-failed" ? (
        <View style={styles.autoBlock}>
          <Text style={[typeScale.sub, { color: tokens.danger }]}>{desktopState.message}</Text>
          <Button label="Try again" variant="secondary" onPress={() => void onStartServerPress()} />
        </View>
      ) : null}

      <View style={[styles.qrWrap, !isCompact && styles.qrWrapWide]}>
        <QRScan onScanned={onScanned} enabled={scanEnabled} paused={busy} />
        <Pressable
          onPress={() => setScanEnabled((enabled) => !enabled)}
          accessibilityRole="button"
          accessibilityLabel={scanEnabled ? "Stop scanning" : "Scan QR code"}
          style={styles.scanLink}
        >
          <Text style={[typeScale.bodyBold, { color: tokens.accent }]}>{scanEnabled ? "Stop scanning" : "Scan QR code"}</Text>
        </Pressable>
      </View>

      <View style={[styles.urlBlock, !isCompact && styles.urlBlockWide]}>
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
          style={styles.connectButton}
        />
      </View>

      {testState !== "idle" && testState !== "testing" && testState !== "ok" ? (
        <Banner tone="danger" message={STATE_COPY[testState]} />
      ) : null}

      {testState === "ok" ? (
        <Text style={[typeScale.body, styles.successText, { color: tokens.success }]}>
          connected — opening forge…
        </Text>
      ) : null}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space20 },
  contentWide: { alignItems: "center" },
  closeRow: { flexDirection: "row", justifyContent: "flex-end", width: "100%" },
  hero: { gap: space.space12, maxWidth: 480, width: "100%" },
  heroTitleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  steps: { gap: space.space8, maxWidth: 480, width: "100%" },
  stepRow: { flexDirection: "row", gap: space.space8 },
  stepIndex: { width: 14 },
  stepText: { flex: 1 },
  autoRow: { flexDirection: "row", alignItems: "center", gap: space.space8, maxWidth: 480, width: "100%" },
  autoText: { flex: 1 },
  autoHint: { maxWidth: 480, width: "100%" },
  autoBlock: { gap: space.space8, maxWidth: 480, width: "100%" },
  qrWrap: { alignItems: "center", gap: space.space12, width: "100%" },
  qrWrapWide: { maxWidth: 480 },
  scanLink: { minHeight: 44, alignItems: "center", justifyContent: "center", paddingHorizontal: space.space16 },
  urlBlock: { gap: space.space4, width: "100%" },
  urlBlockWide: { maxWidth: 480 },
  connectButton: { marginTop: space.space8 },
  successText: { textAlign: "center" },
});
