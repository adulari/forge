// Forge Anywhere — Devices (mobile.dc.html "AW Jobs Shares Devices" lines 563-569,
// desktop.dc.html "D AW Settings" devices column lines 1009-1013). Real device list
// and revoke-with-key-rotation flow — same `anywhere.revokeDevice` used by
// recovery-phrase.tsx's Recovery Center, just given its own dedicated list here.
import { router } from "expo-router";
import { Laptop, Smartphone, Trash2 } from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

function isPhoneLike(name: string): boolean {
  return /phone|android/i.test(name);
}

function relativeLabel(iso: string | null): string {
  if (!iso) return "enrolled";
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return "enrolled";
  const deltaSec = Math.max(0, Math.round((Date.now() - ms) / 1000));
  if (deltaSec < 60) return `${deltaSec}s`;
  const deltaMin = Math.round(deltaSec / 60);
  if (deltaMin < 60) return `${deltaMin}m`;
  const deltaHour = Math.round(deltaMin / 60);
  if (deltaHour < 24) return `${deltaHour}h`;
  const deltaDay = Math.round(deltaHour / 24);
  return `${deltaDay}d`;
}

export default function AnywhereDevicesScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [target, setTarget] = useState<string | null>(null);
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);

  const revoke = useCallback(async () => {
    if (!target || !phrase.trim()) return;
    setBusy(true);
    try {
      await anywhere.revokeDevice(target, phrase);
      toast.show("Device revoked and account keys rotated.", { tone: "neutral" });
      setTarget(null);
      setPhrase("");
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Device could not be revoked.", { tone: "danger" });
    } finally {
      setBusy(false);
    }
  }, [anywhere, phrase, target, toast]);

  if (anywhere.phase !== "ready") return null;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <View style={styles.header}>
          <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
          <View style={styles.headerRow}>
            <Text accessibilityRole="header" style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>
              Devices
            </Text>
            <Button label="Pair device" variant="secondary" onPress={() => router.push("/anywhere/pair")} />
          </View>
        </View>

        <View style={[styles.list, { borderTopColor: tokens.border }]}>
          {anywhere.devices.map((device) => {
            const current = device.id === anywhere.credentials?.deviceIdHex;
            return (
              <View key={device.id} style={[styles.row, { borderBottomColor: tokens.hairline }]}>
                {isPhoneLike(device.name) ? (
                  <Smartphone size={18} color={tokens.ink3} />
                ) : (
                  <Laptop size={18} color={tokens.ink3} />
                )}
                <View style={styles.rowCopy}>
                  <View style={styles.nameLine}>
                    <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
                      {device.name}
                    </Text>
                    {current ? (
                      <Text style={[typeScale.meta, styles.thisDeviceTag, { color: tokens.ink3 }]}>THIS DEVICE</Text>
                    ) : null}
                  </View>
                </View>
                <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{relativeLabel(device.last_seen_at)}</Text>
                {!current ? (
                  <Pressable
                    onPress={() => {
                      setTarget(device.id);
                      setPhrase("");
                    }}
                    accessibilityRole="button"
                    accessibilityLabel={`Revoke ${device.name}`}
                    hitSlop={8}
                  >
                    <Trash2 size={16} color={tokens.danger} />
                  </Pressable>
                ) : null}
              </View>
            );
          })}
          {!anywhere.devices.length ? (
            <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>No devices enrolled yet.</Text>
          ) : null}
        </View>

        {target ? (
          <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Confirm device revocation</Text>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              Enter your recovery phrase. Forge rotates the account key atomically before the device is removed.
            </Text>
            <Input
              label="Recovery phrase"
              value={phrase}
              onChangeText={setPhrase}
              multiline
              autoCapitalize="none"
              autoCorrect={false}
            />
            <View style={styles.actions}>
              <Button label="Cancel" variant="ghost" disabled={busy} onPress={() => setTarget(null)} style={styles.action} />
              <Button
                label="Revoke device"
                variant="danger"
                loading={busy}
                disabled={!phrase.trim()}
                onPress={() => void revoke()}
                style={styles.action}
              />
            </View>
          </View>
        ) : null}

        <Banner
          tone="danger"
          message="Lost a device? Revoke = key rotation: tokens & host grants revoked, a new key epoch is re-wrapped to your remaining devices and recovery phrase, then committed atomically."
          style={styles.flushBanner}
        />
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 680, alignSelf: "center" },
  header: { gap: space.space8, marginBottom: space.space4 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headerTitle: { flex: 1 },
  list: { marginTop: space.space12, borderTopWidth: 1 },
  row: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  rowCopy: { flex: 1, gap: 2 },
  nameLine: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  thisDeviceTag: { fontWeight: "600" },
  empty: { paddingVertical: space.space16 },
  revokePanel: { marginTop: space.space16, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 },
  actions: { flexDirection: "row", gap: space.space8 },
  action: { flex: 1 },
  flushBanner: { marginHorizontal: 0, marginTop: space.space20 },
});
