// Forge Anywhere — approve a new device by code or QR (mobile.dc.html "AW Hosts
// Detail Pair" lines 511-521). Backed by the real enrollment inbox: the waiting device
// prints its challenge, this screen reads it, shows the safety code both sides must
// match, and signs the account-key wrap through AnywhereProvider.approvePairing.
// The challenge is never listed by the service, so it has to arrive by scan or paste.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { Check, ScanLine, X } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { QRScan } from "../../components/pairing/QRScan";
import { useAnywhere, type AnywherePairingPreview } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

function useCountdown(expiresAtMs: number): string {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.ceil((expiresAtMs - now) / 1000));
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s left`;
}

function failure(reason: unknown): string {
  const text = reason instanceof Error ? reason.message : String(reason);
  return text.trim() || "That code could not be read.";
}

export default function AnywherePairScreen() {
  const { inspectPairing, approvePairing, refreshPendingApprovals } = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [code, setCode] = useState("");
  const [scanning, setScanning] = useState(false);
  const [preview, setPreview] = useState<AnywherePairingPreview | null>(null);
  const [challenge, setChallenge] = useState("");
  const [approved, setApproved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submitCode = useCallback(
    async (value: string) => {
      const trimmed = value.trim();
      if (!trimmed) return;
      setBusy(true);
      setError(null);
      try {
        const details = await inspectPairing(trimmed);
        setPreview(details);
        setChallenge(trimmed);
        setScanning(false);
      } catch (reason) {
        setError(failure(reason));
      } finally {
        setBusy(false);
      }
    },
    [inspectPairing],
  );

  const approve = useCallback(async () => {
    if (!challenge || !preview) return;
    setBusy(true);
    setError(null);
    try {
      await approvePairing(challenge);
      setApproved(true);
      toast.show(`${preview.deviceName} approved.`, { tone: "neutral" });
      await refreshPendingApprovals(true);
    } catch (reason) {
      setError(failure(reason));
    } finally {
      setBusy(false);
    }
  }, [approvePairing, challenge, preview, refreshPendingApprovals, toast]);

  const reset = useCallback(() => {
    setPreview(null);
    setChallenge("");
    setApproved(false);
    setError(null);
    setCode("");
  }, []);

  const pasteCode = useCallback(async () => {
    const text = await Clipboard.getStringAsync();
    if (text) setCode(text.trim());
  }, []);

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <BackLink label="Devices" onPress={() => router.replace("/anywhere/devices")} />
        <Text accessibilityRole="header" style={[typeScale.headingBold, styles.title, { color: tokens.ink }]}>
          Approve new device
        </Text>

        {error ? <Banner tone="danger" message={error} style={styles.banner} /> : null}

        {!preview ? (
          <View style={styles.form}>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              Scan the code shown on the new device, or paste it below. Scanning alone grants nothing — review, then
              approve.
            </Text>
            {scanning ? (
              <QRScan enabled onScanned={(data) => void submitCode(data)} />
            ) : (
              <Pressable
                onPress={() => setScanning(true)}
                accessibilityRole="button"
                accessibilityLabel="Scan QR code"
                style={[styles.scanButton, { borderColor: tokens.border }]}
              >
                <ScanLine size={18} color={tokens.accent} />
                <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Scan QR code</Text>
              </Pressable>
            )}
            <Input
              label="Or paste the pairing code"
              value={code}
              onChangeText={setCode}
              autoCapitalize="none"
              autoCorrect={false}
              multiline
              trailing={
                <Pressable onPress={() => void pasteCode()} accessibilityRole="button" accessibilityLabel="Paste">
                  <Text style={[typeScale.meta, { color: tokens.accent }]}>Paste</Text>
                </Pressable>
              }
            />
            <Button label="Continue" onPress={() => void submitCode(code)} loading={busy} disabled={!code.trim()} fullWidth />
          </View>
        ) : (
          <View style={styles.form}>
            <View style={[styles.card, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
              <DetailRow label="Device" value={preview.deviceName} />
              <DetailRow label="Safety code" value={preview.safetyCode} mono />
              {!approved ? <ExpiryRow expiresAtMs={preview.expiresAtMs} /> : null}
            </View>

            {!approved ? (
              <>
                <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
                  The waiting device shows the same safety code. Approve only if they match — approving shares this
                  account&apos;s encrypted history key with that device.
                </Text>
                <View style={styles.actionRow}>
                  <Button
                    label="Approve"
                    variant="allow"
                    icon={<Check size={16} color={tokens.successBg} />}
                    loading={busy}
                    onPress={() => void approve()}
                    style={styles.flexAction}
                  />
                  <Button
                    label="Not now"
                    variant="danger"
                    icon={<X size={16} color={tokens.danger} />}
                    disabled={busy}
                    onPress={reset}
                    style={styles.flexAction}
                  />
                </View>
              </>
            ) : (
              <Banner
                tone="neutral"
                message={`${preview.deviceName} now has account access. It finishes enrolling on its own.`}
              />
            )}

            <Button label="Approve another device" variant="ghost" onPress={reset} fullWidth />
          </View>
        )}

        <Text style={[typeScale.monoMeta, styles.footnote, { color: tokens.ink4 }]}>
          codes expire 10 minutes after the waiting device asks · a code from another account is refused
        </Text>
      </View>
    </Screen>
  );
}

function ExpiryRow({ expiresAtMs }: { expiresAtMs: number }) {
  const remaining = useCountdown(expiresAtMs);
  return <DetailRow label="Expires" value={remaining} />;
}

function DetailRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  const tokens = useTokens();
  return (
    <View style={styles.detailRow}>
      <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{label}</Text>
      <Text
        style={[mono ? typeScale.monoMeta : typeScale.sub, styles.detailValue, { color: tokens.ink2 }]}
        numberOfLines={1}
      >
        {value}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 640, alignSelf: "center" },
  title: { marginTop: space.space12 },
  banner: { marginTop: space.space12 },
  form: { marginTop: space.space20, gap: space.space12 },
  scanButton: {
    minHeight: 96,
    borderWidth: 1,
    borderRadius: radii.radius12,
    borderStyle: "dashed",
    alignItems: "center",
    justifyContent: "center",
    gap: space.space8,
  },
  card: { borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space8 },
  detailRow: { flexDirection: "row", justifyContent: "space-between", gap: space.space12 },
  detailValue: { flex: 1, textAlign: "right" },
  actionRow: { flexDirection: "row", gap: space.space8 },
  flexAction: { flex: 1 },
  footnote: { marginTop: space.space20, lineHeight: 16 },
});
