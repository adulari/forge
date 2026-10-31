// Pairing screen (ARCHITECTURE.md §3 "the daemon contract", §7 App Store review
// posture; FEATURES.md §1.1/§4). Hearth (mobile.dc.html:1181, desktop.dc.html:1105,
// web.dc.html:430): hero + numbered steps, no cards — the QR reticle and the mono
// connect-URL field are the only bordered boxes on this screen. Stands alone for an
// App Store reviewer with no daemon running — explains what Forge is with a bundled,
// no-network "how it works" note before any pairing UI. QR scan is the primary path
// (native camera / web "scan on your phone" hint via the `QRScan.native`/`.web` Metro
// split); the "other ways" sheet covers paste, a photo of the code and manual entry; `?url=` deep links and
// `#token=` page loads go to the same confirm card (sanitised host, cleartext warning) before connecting.
import { router, useLocalSearchParams } from "expo-router";
import { Flame, X } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Svg, { Path } from "react-native-svg";

import { EntitlementBadge } from "../components/anywhere/EntitlementBadge";
import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { IconButton } from "../components/ds/IconButton";
import { Screen } from "../components/ds/Screen";
import { StatusDot } from "../components/ds/StatusDot";
import { useToast } from "../components/ds/ToastHost";
import { ConfirmTargetCard } from "../components/pairing/ConfirmTargetCard";
import { InstallHint } from "../components/pairing/InstallHint";
import { NearbyHosts } from "../components/pairing/NearbyHosts";
import { AddMachineSheet } from "../components/pairing/AddMachineSheet";
import { OtherWaysSheet, type ManualPrefill } from "../components/pairing/OtherWaysSheet";
import { QRScan } from "../components/pairing/QRScan";
import { useAnywhere, useAnywhereHosts } from "../lib/anywhere/store";
import { type ConnectTestState, parseConnectUrl, useAuth } from "../lib/auth";
import { describeConnectTarget, type ConnectTarget } from "../lib/connectUrl";
import { readPairingFromLocation, stripPairingFromLocation } from "../lib/pairingLocation";
import {
  detectForgeServe,
  resolveDesktopConnection,
  forgeBinaryAvailable,
  pollForForgeServe,
  startForgeServe,
  type DetectedServeState,
} from "../lib/desktopServe";
import { haptics } from "../lib/haptics";
import { takePendingRoute } from "../lib/pendingRoute";
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
  "cert-changed":
    "this server's certificate changed since it was paired, so it may be a different machine. if you replaced it on purpose, re-pair: run `forge serve pair` on the host and scan the new code.",
  unreachable:
    "server unreachable — is `forge serve` running and on the same network? a link from `forge serve` pins its self-signed certificate on desktop; on phones use `--tailscale` or `--tunnel` for a certificate every device trusts, or `--local` plus a VPN.",
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

// lucide-react-native ships no GitHub mark — this traces the same glyph the design comp
// embeds inline (mobile.dc.html "AW Connect", line 120), colored via the caller's token
// instead of a literal hex so it still follows the "no raw hex" rule.
function GithubMark({ size, color }: { size: number; color: string }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" fill={color}>
      <Path d="M12 2C6.48 2 2 6.58 2 12.25c0 4.53 2.87 8.37 6.84 9.73.5.09.68-.22.68-.49v-1.7c-2.78.62-3.37-1.37-3.37-1.37-.45-1.18-1.11-1.5-1.11-1.5-.9-.63.07-.62.07-.62 1 .07 1.53 1.05 1.53 1.05.89 1.56 2.34 1.11 2.91.85.09-.66.35-1.11.63-1.37-2.22-.26-4.56-1.14-4.56-5.07 0-1.12.39-2.03 1.03-2.75-.1-.26-.45-1.3.1-2.7 0 0 .84-.28 2.75 1.05a9.4 9.4 0 0 1 5 0c1.91-1.33 2.75-1.05 2.75-1.05.55 1.4.2 2.44.1 2.7.64.72 1.03 1.63 1.03 2.75 0 3.94-2.34 4.8-4.57 5.06.36.32.68.94.68 1.9v2.82c0 .27.18.59.69.49A10.06 10.06 0 0 0 22 12.25C22 6.58 17.52 2 12 2z" />
    </Svg>
  );
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
  const { account: anywhereAccount, signedIn: anywhereSignedIn, refresh: refreshAnywhere } = useAnywhere();
  const { hosts: anywhereHosts, loading: anywhereHostsLoading } = useAnywhereHosts();
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

  const [otherOpen, setOtherOpen] = useState(false);
  const [addMachineOpen, setAddMachineOpen] = useState(false);
  const [prefill, setPrefill] = useState<ManualPrefill | null>(null);
  const [confirm, setConfirm] = useState<{ candidate: string; target: ConnectTarget } | null>(null);
  const [scanEnabled, setScanEnabled] = useState(false);
  const [testState, setTestState] = useState<ConnectTestState>("idle");
  const [formatError, setFormatError] = useState<string | null>(null);

  const busy = testState === "testing";

  const requestConnect = useCallback((candidate: string) => {
    setOtherOpen(false);
    setTestState("idle");
    const parsed = parseConnectUrl(candidate);
    if (!parsed) {
      setConfirm(null);
      setFormatError(
        "that doesn't look like a forge connect url — paste it exactly as `forge serve` printed it, or scan its qr code.",
      );
      return;
    }
    setFormatError(null);
    setConfirm({ candidate, target: describeConnectTarget(parsed) });
  }, []);

  useEffect(() => {
    const raw = params.url;
    if (typeof raw !== "string" || raw.length === 0) return;
    if (lastAppliedUrl.current === raw) return;
    lastAppliedUrl.current = raw;
    requestConnect(decodeParam(raw));
  }, [params.url, requestConnect]);

  useEffect(() => {
    const fromLocation = readPairingFromLocation();
    if (!fromLocation) return;
    stripPairingFromLocation();
    requestConnect(fromLocation);
  }, [requestConnect]);

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
        result = await testConnection(parsed.baseUrl, parsed.fingerprint);
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
          stripPairingFromLocation();
          if (additional) {
            toast.show(`added ${added.name}`, { tone: "success" });
            goBackOr("/(tabs)/settings");
          } else {
            const back = takePendingRoute();
            router.replace((back ?? "/(tabs)") as never);
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
      // Walk the whole ladder — detect, else start, else fall back — rather than parking behind a
      // "Connect" button and then a second "Start server" button. A discoverable local daemon
      // should not need two clicks, and a missing one should not be a dead end with a URL to paste.
      const plan = await resolveDesktopConnection({
        detect: detectForgeServe,
        binaryAvailable: forgeBinaryAvailable,
        start: startForgeServe,
        poll: pollForForgeServe,
      });
      if (cancelled) return;
      switch (plan.kind) {
        case "connect":
          setDesktopState({ kind: "idle" });
          void attemptConnect(plan.state.base_url);
          return;
        case "confirm-lan":
          setDesktopState({ kind: "found-lan", state: plan.state });
          return;
        case "manual":
          setDesktopState(
            plan.reason === "no-binary"
              ? { kind: "unavailable" }
              : {
                  kind: "start-failed",
                  message:
                    plan.message ??
                    "forge serve didn't come up within 15s — check the terminal it prints to, or use other ways to connect.",
                },
          );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [noServersYet, attemptConnect]);

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
          "forge serve didn't come up within 15s — check the terminal it prints to, or use other ways to connect.",
      });
      return;
    }
    setDesktopState({ kind: "idle" });
    void attemptConnect(found.base_url);
  }, [attemptConnect]);

  const onScanned = useCallback((data: string) => requestConnect(data), [requestConnect]);

  const onConfirm = useCallback(() => {
    if (!confirm) return;
    const { candidate } = confirm;
    setConfirm(null);
    void attemptConnect(candidate);
  }, [confirm, attemptConnect]);

  const onCancelConfirm = useCallback(() => {
    setConfirm(null);
    setScanEnabled(false);
  }, []);

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

      {confirm ? (
        <View style={[styles.qrWrap, !isCompact && styles.qrWrapWide]}>
          <ConfirmTargetCard target={confirm.target} busy={busy} onConnect={onConfirm} onCancel={onCancelConfirm} />
        </View>
      ) : (
        <View style={[styles.qrWrap, !isCompact && styles.qrWrapWide]}>
          <Button
            label={scanEnabled ? "Stop scanning" : "Scan pairing code"}
            variant={scanEnabled ? "secondary" : "primary"}
            onPress={() => setScanEnabled((enabled) => !enabled)}
            fullWidth
            accessibilityLabel={scanEnabled ? "Stop scanning" : "Scan pairing code"}
          />
          <QRScan onScanned={onScanned} enabled={scanEnabled} paused={busy} />
          <Button
            label="Other ways to connect"
            variant="ghost"
            onPress={() => setOtherOpen(true)}
            disabled={busy}
            fullWidth
            accessibilityLabel="Other ways to connect"
          />
          {formatError ? <Banner tone="danger" message={formatError} /> : null}
        </View>
      )}

      <NearbyHosts
        onPick={(host) => {
          setPrefill({ host: host.host, port: String(host.port), fingerprint: host.fingerprint ?? undefined });
          setOtherOpen(true);
        }}
      />

      <OtherWaysSheet visible={otherOpen} onClose={() => setOtherOpen(false)} onSubmit={requestConnect} prefill={prefill} />

      <InstallHint />

      {servers.length > 0 ? (
        <View style={[styles.qrWrap, !isCompact && styles.qrWrapWide]}>
          <Button
            label="Add a machine over SSH"
            variant="ghost"
            onPress={() => setAddMachineOpen(true)}
            disabled={busy}
            fullWidth
            accessibilityLabel="Add a machine over SSH"
          />
          <AddMachineSheet visible={addMachineOpen} onClose={() => setAddMachineOpen(false)} />
        </View>
      ) : null}

      {testState !== "idle" && testState !== "testing" && testState !== "ok" ? (
        <View style={[styles.qrWrap, !isCompact && styles.qrWrapWide]}>
          <Banner tone="danger" message={STATE_COPY[testState]} />
        </View>
      ) : null}

      {testState === "ok" ? (
        <Text style={[typeScale.body, styles.successText, { color: tokens.success }]}>
          connected — opening forge…
        </Text>
      ) : null}

      {/* Forge Anywhere entry point — mobile.dc.html "AW Connect" (lines 116-148). The
          direct pairing flow above stays untouched; this section only adds the optional
          managed-relay path below it. */}
      <View style={styles.anywhereSection}>
        <Text style={[typeScale.section, { color: tokens.ink4 }]}>anywhere</Text>
        <Text style={[typeScale.sub, styles.anywhereDescription, { color: tokens.ink2 }]}>
          Optional managed encrypted relay. Leave your desk without leaving your session; Direct, LAN,
          and your own tunnels stay free.
        </Text>

        {anywhereSignedIn && anywhereAccount ? (
          <>
            <Pressable
              onPress={() => router.push("/anywhere")}
              accessibilityRole="button"
              accessibilityLabel="Forge Anywhere"
              style={styles.anywhereStatusRow}
            >
              <EntitlementBadge account={anywhereAccount} />
              <Text style={[typeScale.monoMeta, styles.anywhereStatusMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                {`@${anywhereAccount.githubLogin} · ${anywhereHostsLoading ? "…" : `${anywhereHosts.length} host${anywhereHosts.length === 1 ? "" : "s"}`}`}
              </Text>
            </Pressable>

            {!anywhereAccount.relayConnected ? (
              <View style={styles.anywhereStateRow}>
                <Text style={[typeScale.sub, styles.anywhereStateText, { color: tokens.ink2 }]}>
                  Anywhere relay unreachable — Direct still works
                </Text>
                <Pressable onPress={() => void refreshAnywhere()} accessibilityRole="button" accessibilityLabel="Retry" hitSlop={8}>
                  <Text style={[typeScale.sub, styles.anywhereActionText, { color: tokens.accent }]}>Retry</Text>
                </Pressable>
              </View>
            ) : !anywhereHostsLoading && anywhereHosts.length === 0 ? (
              <View style={styles.anywhereStateRow}>
                <Text style={[typeScale.sub, styles.anywhereStateText, { color: tokens.ink2 }]}>
                  Signed in, no host enrolled yet
                </Text>
                <Pressable
                  onPress={() => router.push("/anywhere/first-host")}
                  accessibilityRole="button"
                  accessibilityLabel="Connect a host"
                  hitSlop={8}
                >
                  <Text style={[typeScale.sub, styles.anywhereActionText, { color: tokens.accent }]}>Connect a host</Text>
                </Pressable>
              </View>
            ) : null}
          </>
        ) : (
          <>
            <Button
              label="Sign in with GitHub"
              icon={<GithubMark size={16} color={tokens.onAccent} />}
              onPress={() => router.push("/anywhere/sign-in")}
              fullWidth
              style={styles.anywhereButton}
            />
            <Text style={[typeScale.sub, styles.anywhereFootnote, { color: tokens.ink4 }]}>
              EUR 10/month · 14-day trial, no card · one Forge app, two transports
            </Text>
          </>
        )}
      </View>

      <Pressable
        onPress={() => router.push("/legal")}
        accessibilityRole="link"
        accessibilityLabel="Privacy, license, and support"
        hitSlop={8}
      >
        <Text style={[typeScale.sub, styles.legalLink, { color: tokens.ink3 }]}>
          Privacy · license · support
        </Text>
      </Pressable>
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
  successText: { textAlign: "center" },
  anywhereSection: { gap: space.space4, maxWidth: 480, width: "100%" },
  anywhereDescription: { marginBottom: space.space4 },
  anywhereButton: { marginTop: space.space4 },
  anywhereFootnote: { textAlign: "center", marginTop: space.space4 },
  anywhereStatusRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8, minHeight: 44 },
  anywhereStatusMeta: { flex: 1 },
  anywhereStateRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8 },
  anywhereStateText: { flex: 1 },
  anywhereActionText: { fontWeight: "600" },
  legalLink: { textAlign: "center", paddingVertical: space.space8 },
});
