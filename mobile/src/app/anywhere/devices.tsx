// Forge Anywhere — Devices + key rotation (mobile.dc.html "AW Devices" lines 1079-1131).
// The design shows one hardcoded "Revoke MacBook Pro" card; here every non-this-device row
// opens the same rotation sheet targeted at whichever device was tapped (task spec: "row
// tap open a revoke flow sheet") rather than a single static target.
import { router } from "expo-router";
import { Check, ChevronRight, Laptop, Smartphone, Tablet } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { SettingsShell } from "../(tabs)/settings";
import { Badge } from "../../components/ds/Badge";
import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Sheet } from "../../components/ds/Sheet";
import { SkeletonRow } from "../../components/ds/Skeleton";
import { useToast } from "../../components/ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { goBackOr } from "../../lib/nav";
import { useAnywhere, useAnywhereDevices } from "../../lib/anywhere/store";
import type { AnywhereDevice, DeviceKind, RotationStep } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { formatRelativeTime, type as typeScale, tabularNums } from "../../theme/typography";

const ROTATION_ORDER: RotationStep[] = ["revoking-tokens", "creating-key-epoch", "wrapping-keys", "committing", "done"];
const MIN_PHRASE_WORDS = 24;

/** A named component (not a variable holding a component type) so JSX below never
 * "creates a component during render" — react-hooks/static-components requires this. */
function DeviceKindIcon({ kind, color }: { kind: DeviceKind; color: string }) {
  if (kind === "tablet") return <Tablet size={18} strokeWidth={1.75} color={color} />;
  if (kind === "laptop") return <Laptop size={18} strokeWidth={1.75} color={color} />;
  return <Smartphone size={18} strokeWidth={1.75} color={color} />;
}

function DeviceRow({ device, onPress, showSeparator }: { device: AnywhereDevice; onPress?: () => void; showSeparator: boolean }) {
  const tokens = useTokens();
  const metaText = device.isThisDevice
    ? `enrolled ${formatRelativeTime(device.enrolledAt)} ago`
    : `enrolled ${formatRelativeTime(device.enrolledAt)} ago · last seen ${formatRelativeTime(device.lastSeenAt)} ago`;

  const content = (
    <View style={styles.deviceRow}>
      <DeviceKindIcon kind={device.kind} color={tokens.ink3} />
      <View style={styles.deviceBody}>
        <View style={styles.deviceNameRow}>
          <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
            {device.name}
          </Text>
          {device.isThisDevice ? <Badge label="THIS DEVICE" tone="accent" /> : null}
        </View>
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          {device.fingerprint}
        </Text>
      </View>
      <Text style={[typeScale.monoMeta, tabularNums, styles.deviceMeta, { color: tokens.ink3 }]} numberOfLines={1}>
        {metaText}
      </Text>
      {onPress ? <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink4} /> : null}
    </View>
  );

  return (
    <View>
      {onPress ? (
        <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={`Revoke ${device.name}`}>
          {content}
        </Pressable>
      ) : (
        content
      )}
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

interface RotationItemState {
  label: string;
  status: "done" | "current" | "pending";
}

function rotationItems(step: RotationStep | null, remainingDevices: number): RotationItemState[] {
  const currentIndex = step ? ROTATION_ORDER.indexOf(step) : -1;
  const labels = [
    "Tokens & host grants revoked",
    "Creating key epoch…",
    `Wrap to ${remainingDevices} device${remainingDevices === 1 ? "" : "s"} + recovery phrase`,
    "Commit",
  ];
  return labels.map((label, i) => ({
    label,
    status: i <= currentIndex ? "done" : i === currentIndex + 1 && step != null ? "current" : "pending",
  }));
}

function RotationStepRow({ item }: { item: RotationItemState }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(item.status === "current" ? "busy" : "idle");
  return (
    <View style={styles.stepRow}>
      {item.status === "done" ? (
        <View style={[styles.stepGlyph, { backgroundColor: tokens.successBg }]}>
          <Check size={8} strokeWidth={3.5} color={tokens.success} />
        </View>
      ) : item.status === "current" ? (
        <View style={[styles.stepGlyph, { backgroundColor: tokens.selection }]}>
          <Animated.View style={[styles.stepPulseDot, { backgroundColor: tokens.accent }, dotStyle]} />
        </View>
      ) : (
        <View style={[styles.stepGlyph, styles.stepGlyphPending, { borderColor: tokens.borderStrong }]} />
      )}
      <Text style={[typeScale.sub, { color: item.status === "pending" ? tokens.ink3 : tokens.ink2 }]}>{item.label}</Text>
    </View>
  );
}

export default function AnywhereDevicesScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { client, signedIn, loading: accountLoading } = useAnywhere();
  const { devices, loading, refresh } = useAnywhereDevices();

  const [target, setTarget] = useState<AnywhereDevice | null>(null);
  const [phrase, setPhrase] = useState("");
  const [rotationStep, setRotationStep] = useState<RotationStep | null>(null);
  const [rotating, setRotating] = useState(false);
  const [rotationError, setRotationError] = useState<string | null>(null);

  useEffect(() => {
    if (!accountLoading && !signedIn) router.replace("/anywhere");
  }, [accountLoading, signedIn]);

  const openRotation = useCallback((device: AnywhereDevice) => {
    setTarget(device);
    setPhrase("");
    setRotationStep(null);
    setRotationError(null);
  }, []);

  const closeRotation = useCallback(() => {
    if (rotating) return;
    setTarget(null);
  }, [rotating]);

  const phraseWordCount = phrase.trim().split(/\s+/).filter(Boolean).length;
  const phraseValid = phraseWordCount >= MIN_PHRASE_WORDS;

  const onConfirmRotate = useCallback(async () => {
    if (!target || !phraseValid) return;
    setRotating(true);
    setRotationError(null);
    try {
      await client.revokeDeviceAndRotate(target.id, phrase, (step) => setRotationStep(step));
      await refresh();
      haptics.pairSuccess();
      toast.show(`${target.name} revoked · keys rotated.`, { tone: "neutral" });
      setTarget(null);
    } catch {
      setRotationStep(null);
      setRotationError("Recovery phrase looks wrong — nothing changed.");
      haptics.mergeConflict();
    } finally {
      setRotating(false);
    }
  }, [client, target, phrase, phraseValid, refresh, toast]);

  if (!signedIn) return null;

  const remainingAfterRevoke = Math.max(0, devices.length - 1); // every device except the one being revoked

  return (
    <SettingsShell active="anywhere">
      <Screen scroll contentContainerStyle={styles.content}>
        <View style={styles.headerRow}>
          <BackLink label="Anywhere" onPress={() => goBackOr("/anywhere")} />
          <Text style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>Devices</Text>
          <Pressable
            onPress={() => router.push("/anywhere/pair")}
            accessibilityRole="button"
            accessibilityLabel="Pair device"
          >
            <Text style={[typeScale.bodyBold, { color: tokens.accent }]}>Pair device</Text>
          </Pressable>
        </View>

        {loading ? (
          <View style={styles.skeletonWrap}>
            <SkeletonRow />
            <SkeletonRow />
          </View>
        ) : (
          <View style={styles.section}>
            {devices.map((device, index) => (
              <DeviceRow
                key={device.id}
                device={device}
                showSeparator={index < devices.length - 1}
                onPress={device.isThisDevice ? undefined : () => openRotation(device)}
              />
            ))}
          </View>
        )}

        <View style={styles.section}>
          <SectionHeader>Lost a device? — key rotation</SectionHeader>
          <Text style={[typeScale.sub, styles.lostCopy, { color: tokens.ink3 }]}>
            Tap a device above to revoke it — Forge revokes its tokens and hosts, creates a new key epoch, re-wraps
            to your remaining devices and recovery phrase, then commits, atomically.
          </Text>
        </View>
      </Screen>

      <Sheet visible={target != null} onClose={closeRotation} accessibilityLabel="Revoke device and rotate keys">
        {target ? (
          <View style={styles.sheetContent}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>{`Revoke ${target.name}`}</Text>
            <Text style={[typeScale.sub, styles.sheetIntro, { color: tokens.ink2 }]}>
              This is more than delete: Forge revokes its tokens and hosts, creates a new key epoch, re-wraps to
              your remaining devices and recovery phrase, then commits — atomically.
            </Text>

            <View style={styles.stepsWrap}>
              {rotationItems(rotationStep, remainingAfterRevoke).map((item) => (
                <RotationStepRow key={item.label} item={item} />
              ))}
            </View>

            <Text style={[typeScale.meta, styles.sheetFootnote, { color: tokens.ink4 }]}>
              {"Requires your recovery phrase — held in memory only. If any step fails, nothing changes. Future data uses the new epoch; the lost device can't read it."}
            </Text>

            {rotationError ? <Banner tone="danger" message={rotationError} /> : null}

            <Input
              label="Recovery phrase"
              mono
              multiline
              value={phrase}
              onChangeText={setPhrase}
              editable={!rotating}
              placeholder="24-word recovery phrase"
              accessibilityLabel="Recovery phrase"
              containerStyle={styles.phraseInput}
            />

            <View style={styles.sheetActions}>
              <Pressable
                onPress={onConfirmRotate}
                disabled={!phraseValid || rotating}
                accessibilityRole="button"
                accessibilityLabel="Revoke and rotate"
                style={[styles.rotateButton, { backgroundColor: tokens.dangerBg, opacity: !phraseValid || rotating ? 0.5 : 1 }]}
              >
                <Text style={[typeScale.bodyBold, { color: tokens.danger }]}>
                  {rotating ? "Rotating…" : "Revoke & rotate"}
                </Text>
              </Pressable>
              <Pressable
                onPress={closeRotation}
                disabled={rotating}
                accessibilityRole="button"
                accessibilityLabel="Cancel"
                style={[styles.cancelButton, { borderColor: tokens.borderStrong, opacity: rotating ? 0.5 : 1 }]}
              >
                <Text style={[typeScale.bodyBold, { color: tokens.ink2 }]}>Cancel</Text>
              </Pressable>
            </View>
          </View>
        ) : null}
      </Sheet>
    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space4 },
  headerTitle: { flex: 1 },
  section: { marginTop: space.space20 },
  skeletonWrap: { marginTop: space.space16, gap: space.space8 },
  deviceRow: { flexDirection: "row", alignItems: "center", gap: 10, minHeight: 56, paddingHorizontal: space.space4 },
  deviceBody: { flex: 1, minWidth: 0, gap: 2 },
  deviceNameRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  deviceMeta: { flexShrink: 0 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
  lostCopy: { marginTop: space.space8, lineHeight: 18 },
  sheetContent: { padding: space.space20, gap: space.space4 },
  sheetIntro: { lineHeight: 18, marginTop: space.space4 },
  stepsWrap: { marginTop: space.space12, gap: space.space8 },
  stepRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  stepGlyph: { width: 15, height: 15, borderRadius: 8, alignItems: "center", justifyContent: "center" },
  stepGlyphPending: { borderWidth: 1.5, backgroundColor: "transparent" },
  stepPulseDot: { width: 6, height: 6, borderRadius: 3 },
  sheetFootnote: { marginTop: space.space8, lineHeight: 15 },
  phraseInput: { marginTop: space.space12 },
  sheetActions: { flexDirection: "row", gap: space.space8, marginTop: space.space12 },
  rotateButton: { flex: 1, minHeight: tapTarget - 8, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center" },
  cancelButton: { flex: 1, minHeight: tapTarget - 8, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center", borderWidth: StyleSheet.hairlineWidth },
});
