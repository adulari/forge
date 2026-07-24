// Forge Anywhere — approve a new device by code or QR (mobile.dc.html "AW Hosts
// Detail Pair" lines 511-521). Backed by the AnywhereClient pairing surface
// (client.startPair/approvePair/rejectPair + PairChallenge) — the real backend has
// no public pairing-challenge-with-fingerprint-and-grants endpoint yet, so this
// flow is intentionally scoped to the client interface rather than bolted onto the
// real device-approval inbox on the Hub (which trades in a different, safety-code
// shape). Structured like passkey.tsx: a single status-driven card.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { Check, KeyRound, ScanLine, X } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { QRScan } from "../../components/pairing/QRScan";
import { useAnywhere } from "../../lib/anywhere/store";
import type { PairChallenge, PairChallengeState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

const STATE_CAPTION: Record<PairChallengeState, string | null> = {
  pending: null,
  approved: "Approved — the device now has account access.",
  rejected: "Rejected. No keys were shared.",
  expired: "Expired — pairing codes are valid for 10 minutes. Ask the device to generate a new one.",
  "already-used": "This code was already used for a different device.",
  "wrong-account": "That code belongs to a different Forge account — blocked.",
  malformed: "That code isn't a valid pairing code.",
  "camera-denied": "Camera access was denied — paste the code instead.",
};

function useCountdown(expiresAtMs: number): string {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.ceil((expiresAtMs - now) / 1000));
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s left`;
}

export default function AnywherePairScreen() {
  const { client } = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [code, setCode] = useState("");
  const [scanning, setScanning] = useState(false);
  const [challenge, setChallenge] = useState<PairChallenge | null>(null);
  const [busy, setBusy] = useState(false);

  const submitCode = useCallback(
    async (value: string) => {
      if (!value.trim()) return;
      setBusy(true);
      try {
        const result = await client.startPair(value.trim());
        setChallenge(result);
        setScanning(false);
      } finally {
        setBusy(false);
      }
    },
    [client],
  );

  const approve = useCallback(async () => {
    if (!challenge) return;
    setBusy(true);
    try {
      await client.approvePair(challenge.id);
      setChallenge({ ...challenge, state: "approved" });
      toast.show(`${challenge.deviceName} approved.`, { tone: "neutral" });
    } finally {
      setBusy(false);
    }
  }, [challenge, client, toast]);

  const reject = useCallback(async () => {
    if (!challenge) return;
    setBusy(true);
    try {
      await client.rejectPair(challenge.id);
      setChallenge({ ...challenge, state: "rejected" });
      toast.show("Device rejected.", { tone: "neutral" });
    } finally {
      setBusy(false);
    }
  }, [challenge, client, toast]);

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

        {!challenge ? (
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
              autoCapitalize="characters"
              autoCorrect={false}
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
              <DetailRow label="Device" value={`${challenge.deviceName} · ${challenge.deviceKind}`} />
              <DetailRow label="Fingerprint" value={challenge.fingerprint} mono />
              <DetailRow label="Grants" value={challenge.grants.join(", ")} />
              {challenge.state === "pending" ? <ExpiryRow expiresAt={challenge.expiresAt} /> : null}
            </View>

            {challenge.state === "pending" ? (
              <View style={styles.actionRow}>
                <Button label="Approve" variant="allow" icon={<Check size={16} color={tokens.successBg} />} loading={busy} onPress={() => void approve()} style={styles.flexAction} />
                <Button label="Reject" variant="danger" icon={<X size={16} color={tokens.danger} />} disabled={busy} onPress={() => void reject()} style={styles.flexAction} />
              </View>
            ) : (
              <View style={[styles.resultBanner, { borderColor: tokens.border }]}>
                {challenge.state === "approved" ? (
                  <KeyRound size={18} color={tokens.success} />
                ) : (
                  <X size={18} color={tokens.danger} />
                )}
                <Text style={[typeScale.sub, styles.resultText, { color: tokens.ink2 }]}>
                  {STATE_CAPTION[challenge.state]}
                </Text>
              </View>
            )}

            <Button
              label="Pair another device"
              variant="ghost"
              onPress={() => {
                setChallenge(null);
                setCode("");
              }}
              fullWidth
            />
          </View>
        )}

        <Text style={[typeScale.monoMeta, styles.footnote, { color: tokens.ink4 }]}>
          states: expired (10-min window) · already used · wrong account — blocked
        </Text>
      </View>
    </Screen>
  );
}

function ExpiryRow({ expiresAt }: { expiresAt: number }) {
  const remaining = useCountdown(expiresAt);
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
  resultBanner: { flexDirection: "row", alignItems: "center", gap: space.space8, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16 },
  resultText: { flex: 1, lineHeight: 18 },
  footnote: { marginTop: space.space20, lineHeight: 16 },
});
