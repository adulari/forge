// Forge Anywhere — Approve new device (mobile.dc.html "AW Pair Device" lines 293-342).
// Scanning/pasting only fetches the challenge (`client.startPair`) for review — nothing
// is granted until Approve. The design's "challenge states" block (lines 322-333) is a
// reference legend for every `PairChallengeState`; this screen renders whichever state
// `startPair` actually returns instead of a static showcase.
import { router } from "expo-router";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { SettingsShell } from "../(tabs)/settings";
import { Banner, type BannerTone } from "../../components/ds/Banner";
import { BackLink } from "../../components/ds/BackLink";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { QRScan } from "../../components/pairing/QRScan";
import { haptics } from "../../lib/haptics";
import { goBackOr } from "../../lib/nav";
import { useAnywhere } from "../../lib/anywhere/store";
import type { PairChallenge, PairChallengeState } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type as typeScale, tabularNums } from "../../theme/typography";

// `nowMs` defaults via a plain function param (mirrors theme/typography.ts's
// formatRelativeTime) rather than a direct `Date.now()` call inside a component body —
// keeps the render function itself pure per the "components must be idempotent" rule.
function formatCountdown(expiresAt: number, nowMs: number = Date.now()): string {
  const totalSec = Math.max(0, Math.ceil((expiresAt - nowMs) / 1000));
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}m ${s.toString().padStart(2, "0")}s left`;
}

interface ChallengeStatusInfo {
  tone: BannerTone;
  message: string;
  actionLabel?: string;
}

function challengeStatusInfo(state: PairChallengeState): ChallengeStatusInfo | null {
  switch (state) {
    case "pending":
    case "approved":
      return null;
    case "rejected":
      return { tone: "neutral", message: "Request rejected." };
    case "expired":
      return { tone: "warn", message: "The 10-minute window passed for this code.", actionLabel: "Ask for a new code" };
    case "already-used":
      return { tone: "neutral", message: "This code was already used." };
    case "wrong-account":
      return { tone: "danger", message: "This code is for a different account — blocked." };
    case "malformed":
      return { tone: "warn", message: "That code couldn't be read.", actionLabel: "Paste instead" };
    case "camera-denied":
      return { tone: "warn", message: "Camera access is off.", actionLabel: "Paste code" };
    default: {
      const _exhaustive: never = state;
      return _exhaustive;
    }
  }
}

function PulsingBeacon() {
  const tokens = useTokens();
  const { dotStyle, ringStyle } = useEmberdot("waiting");
  return (
    <View style={styles.beaconWrap}>
      <Animated.View style={[styles.beaconRing, { borderColor: tokens.danger }, ringStyle]} />
      <Animated.View style={[styles.beaconDot, { backgroundColor: tokens.danger }, dotStyle]} />
    </View>
  );
}

function ChallengeRow({ label, value }: { label: string; value: string }) {
  const tokens = useTokens();
  return (
    <View style={styles.challengeRow}>
      <Text style={[typeScale.meta, styles.challengeLabel, { color: tokens.ink3 }]}>{label}</Text>
      <Text style={[typeScale.meta, { color: tokens.ink }]} numberOfLines={1}>
        {value}
      </Text>
    </View>
  );
}

function PairChallengeCard({
  challenge,
  onApprove,
  onReject,
  busy,
}: {
  challenge: PairChallenge;
  onApprove: () => void;
  onReject: () => void;
  busy: boolean;
}) {
  const tokens = useTokens();
  // Ticks once a second to force a re-render; formatCountdown reads the actual clock
  // itself (via its own default param) rather than this component sampling Date.now().
  const [, tick] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => tick((v) => v + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  return (
    <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <View style={[styles.cardEdge, { backgroundColor: tokens.danger }]} />
      <View style={styles.cardHeaderRow}>
        <PulsingBeacon />
        <Text style={[typeScale.bodyBold, styles.cardTitle, { color: tokens.ink }]} numberOfLines={1}>
          {`${challenge.deviceName} wants to join`}
        </Text>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
          {formatCountdown(challenge.expiresAt)}
        </Text>
      </View>

      <View style={styles.challengeRows}>
        <ChallengeRow label="Account" value={`@${challenge.account}`} />
        <ChallengeRow label="Device" value={`${challenge.deviceName} · ${challenge.deviceKind}`} />
        <ChallengeRow label="Fingerprint" value={challenge.fingerprint} />
        <ChallengeRow label="Grants" value={challenge.grants.join(", ")} />
      </View>

      <View style={styles.cardActions}>
        <Pressable
          onPress={onApprove}
          disabled={busy}
          accessibilityRole="button"
          accessibilityLabel={`Approve ${challenge.deviceName}`}
          style={[styles.actionButton, { backgroundColor: tokens.successBg, opacity: busy ? 0.6 : 1 }]}
        >
          <Text style={[typeScale.bodyBold, { color: tokens.success }]}>Approve</Text>
        </Pressable>
        <Pressable
          onPress={onReject}
          disabled={busy}
          accessibilityRole="button"
          accessibilityLabel={`Reject ${challenge.deviceName}`}
          style={[styles.actionButton, styles.rejectButton, { borderColor: tokens.borderStrong, opacity: busy ? 0.6 : 1 }]}
        >
          <Text style={[typeScale.bodyBold, { color: tokens.danger }]}>Reject</Text>
        </Pressable>
      </View>
    </View>
  );
}

export default function PairDeviceScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { client, signedIn, loading: accountLoading } = useAnywhere();
  const [challenge, setChallenge] = useState<PairChallenge | null>(null);
  const [pasteValue, setPasteValue] = useState("");
  const [starting, setStarting] = useState(false);
  const [acting, setActing] = useState(false);

  useEffect(() => {
    if (!accountLoading && !signedIn) router.replace("/anywhere");
  }, [accountLoading, signedIn]);

  const startPair = useCallback(
    async (codeOrScan: string) => {
      if (starting || !codeOrScan.trim()) return;
      setStarting(true);
      try {
        const next = await client.startPair(codeOrScan.trim());
        setChallenge(next);
      } catch {
        toast.show("Couldn't read that code — try pasting it instead.", { tone: "danger" });
      } finally {
        setStarting(false);
      }
    },
    [client, starting, toast],
  );

  const onApprove = useCallback(async () => {
    if (!challenge) return;
    setActing(true);
    try {
      await client.approvePair(challenge.id);
      haptics.pairSuccess();
      toast.show(`${challenge.deviceName} approved and paired.`, { tone: "neutral" });
      router.replace("/anywhere/devices");
    } finally {
      setActing(false);
    }
  }, [client, challenge, toast]);

  const onReject = useCallback(async () => {
    if (!challenge) return;
    setActing(true);
    try {
      await client.rejectPair(challenge.id);
      haptics.deny();
      setChallenge({ ...challenge, state: "rejected" });
    } finally {
      setActing(false);
    }
  }, [client, challenge]);

  const statusInfo = challenge ? challengeStatusInfo(challenge.state) : null;

  if (!signedIn) return null;

  return (
    <SettingsShell active="anywhere">
      <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
        <BackLink label="Devices" onPress={() => goBackOr("/anywhere/devices")} />
        <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]}>Approve new device</Text>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          Scan the code shown on the new device, or paste it below. Scanning alone grants nothing — review, then
          approve.
        </Text>

        {challenge && challenge.state === "pending" ? (
          <View style={styles.cardSlot}>
            <PairChallengeCard challenge={challenge} onApprove={onApprove} onReject={onReject} busy={acting} />
          </View>
        ) : (
          <View style={styles.scanSlot}>
            <QRScan onScanned={startPair} enabled={!challenge} paused={starting} />
          </View>
        )}

        {statusInfo ? (
          <Banner
            tone={statusInfo.tone}
            message={statusInfo.message}
            actionLabel={statusInfo.actionLabel}
            onAction={statusInfo.actionLabel ? () => setChallenge(null) : undefined}
          />
        ) : null}

        <View style={styles.pasteSlot}>
          <Input
            mono
            placeholder="paste pairing code…"
            value={pasteValue}
            onChangeText={setPasteValue}
            onSubmitEditing={() => void startPair(pasteValue)}
            returnKeyType="done"
            accessibilityLabel="Paste pairing code"
            editable={!challenge || challenge.state !== "pending"}
          />
        </View>
      </Screen>
    </SettingsShell>
  );
}

const BEACON = 8;

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48 },
  title: { paddingHorizontal: space.space4, marginTop: space.space8 },
  subtitle: { paddingHorizontal: space.space4, marginTop: space.space4, lineHeight: 19 },
  scanSlot: { marginTop: space.space20, alignItems: "center" },
  cardSlot: { marginTop: space.space20 },
  pasteSlot: { marginTop: space.space16 },
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space16, overflow: "hidden" },
  cardEdge: { position: "absolute", left: 0, top: 14, bottom: 14, width: 2, borderRadius: 1 },
  cardHeaderRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginLeft: space.space8 },
  cardTitle: { flex: 1 },
  beaconWrap: { width: BEACON, height: BEACON, alignItems: "center", justifyContent: "center" },
  beaconDot: { width: BEACON, height: BEACON, borderRadius: BEACON / 2, position: "absolute" },
  beaconRing: { width: BEACON, height: BEACON, borderRadius: BEACON / 2, borderWidth: 1.5, position: "absolute" },
  challengeRows: { marginTop: space.space12, marginLeft: space.space16, gap: 7 },
  challengeRow: { flexDirection: "row", gap: 10 },
  challengeLabel: { width: 86 },
  cardActions: { flexDirection: "row", gap: space.space8, marginTop: space.space12, marginLeft: space.space16 },
  actionButton: { flex: 1, minHeight: tapTarget - 6, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center" },
  rejectButton: { backgroundColor: "transparent", borderWidth: StyleSheet.hairlineWidth },
});
