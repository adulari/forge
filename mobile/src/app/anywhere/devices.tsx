// Forge Anywhere — Devices (mobile.dc.html "AW Jobs Shares Devices" lines 563-569,
// desktop.dc.html "D AW Settings" devices column lines 1009-1013). Real device list
// and revoke-with-key-rotation flow — same `anywhere.revokeDevice` used by
// recovery-phrase.tsx's Recovery Center, just given its own dedicated list here.
import { PAGE_MAX_WIDTH } from "../../components/ds/pageWidth";
import { Crown, Laptop, Smartphone, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Badge } from "../../components/ds/Badge";
import { useClockSeconds } from "../../components/ds/relativeClock";
import { canMakeOwner, describeDevices } from "../../lib/anywhere/deviceList";
import { PageHeader } from "../../components/ds/PageHeader";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { DEFAULT_ANYWHERE_SERVICE_URL, fetchAnywhereCapabilities } from "../../lib/anywhereApi";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { goBackOr } from "../../lib/nav";

export default function AnywhereDevicesScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [target, setTarget] = useState<string | null>(null);
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [ownerTarget, setOwnerTarget] = useState<string | null>(null);
  const [ownerTransferSupported, setOwnerTransferSupported] = useState(false);
  const nowSeconds = useClockSeconds();
  const serviceUrl = anywhere.credentials?.serviceUrl;
  useEffect(() => {
    let cancelled = false;
    fetchAnywhereCapabilities(serviceUrl ?? process.env.EXPO_PUBLIC_FORGE_ANYWHERE_URL ?? DEFAULT_ANYWHERE_SERVICE_URL)
      .then((capabilities) => { if (!cancelled) setOwnerTransferSupported(capabilities.features.owner_transfer === true); })
      .catch(() => { if (!cancelled) setOwnerTransferSupported(false); });
    return () => { cancelled = true; };
  }, [serviceUrl]);
  const deviceRows = useMemo(
    () => describeDevices(anywhere.devices, anywhere.credentials?.deviceIdHex, nowSeconds * 1000),
    [anywhere.devices, anywhere.credentials?.deviceIdHex, nowSeconds],
  );

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

  const thisDeviceIsOwner = deviceRows.some((row) => row.isThisDevice && row.isOwner);

  const makeOwner = useCallback(async () => {
    if (!ownerTarget) return;
    setBusy(true);
    try {
      await anywhere.makeOwner(ownerTarget);
      toast.show("Ownership handed over.", { tone: "neutral" });
      setOwnerTarget(null);
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Ownership could not be handed over.", { tone: "danger" });
    } finally {
      setBusy(false);
    }
  }, [anywhere, ownerTarget, toast]);

  if (anywhere.phase !== "ready") return null;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <PageHeader
          title="Devices"
          subtitle={`${deviceRows.length} paired. Any of your devices can revoke another; only the owner device can revoke the owner. The account key is rotated so a revoked device cannot read new data.`}
          backLabel="Anywhere"
          onBack={() => goBackOr("/anywhere")}
        />


        <View style={[styles.list, { borderTopColor: tokens.border }]}>
          {deviceRows.map((device) => (
            <View key={device.id} style={[styles.row, { borderBottomColor: tokens.hairline }]}>
              {device.kind === "phone" ? <Smartphone size={18} color={tokens.ink3} /> : <Laptop size={18} color={tokens.ink3} />}
              <View style={styles.rowCopy}>
                <View style={styles.nameLine}>
                  <Text style={[typeScale.bodyBold, styles.deviceName, { color: tokens.ink }]} numberOfLines={1}>{device.name}</Text>
                  {device.isThisDevice ? <Badge label="THIS DEVICE" tone="accent" /> : null}
                  {device.isOwner ? <Badge label="OWNER" tone="neutral" /> : null}
                  {device.stale ? <Badge label="INACTIVE" tone="warn" /> : null}
                </View>
                <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={2}>{`${device.lastSeen} · ${device.enrolled}`}</Text>
                {device.isOwner && !device.isThisDevice && !device.canRevoke ? (
                  <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={2}>
                    Only the owner device can revoke itself. That keeps a way back in if another device is lost.
                  </Text>
                ) : null}
              </View>
              {canMakeOwner(device, { thisDeviceIsOwner, ownerTransferSupported }) ? (
                <Pressable
                  onPress={() => {
                    setOwnerTarget(device.id);
                    setTarget(null);
                  }}
                  accessibilityRole="button"
                  accessibilityLabel={`Make ${device.name} the owner`}
                  hitSlop={8}
                >
                  <Crown size={16} color={tokens.ink2} />
                </Pressable>
              ) : null}
              {device.canRevoke ? (
                <Pressable
                  onPress={() => {
                    setTarget(device.id);
                    setOwnerTarget(null);
                    setPhrase("");
                  }}
                  accessibilityRole="button"
                  accessibilityLabel={`Revoke ${device.name}, ${device.enrolled.toLowerCase()}`}
                  hitSlop={8}
                >
                  <Trash2 size={16} color={tokens.danger} />
                </Pressable>
              ) : null}
            </View>
          ))}
          {!anywhere.devices.length ? (
            <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>No devices enrolled yet.</Text>
          ) : null}
        </View>

        {ownerTarget ? (
          <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>{`Make ${deviceRows.find((row) => row.id === ownerTarget)?.name ?? "device"} the owner?`}</Text>
            <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
              Only the owner can revoke the owner or hand ownership on. This device stops being the owner.
            </Text>
            <View style={styles.actions}>
              <Button label="Cancel" variant="ghost" disabled={busy} onPress={() => setOwnerTarget(null)} style={styles.action} />
              <Button label="Make owner" variant="primary" loading={busy} onPress={() => void makeOwner()} style={styles.action} />
            </View>
          </View>
        ) : null}

        {target ? (
          <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>{`Revoke ${deviceRows.find((row) => row.id === target)?.name ?? "device"}?`}</Text>
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
  shell: { width: "100%", maxWidth: PAGE_MAX_WIDTH, alignSelf: "center" },
  header: { gap: space.space8, marginBottom: space.space4 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headerTitle: { flex: 1 },
  list: { marginTop: space.space12, borderTopWidth: 1 },
  row: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  rowCopy: { flex: 1, gap: 2 },
  nameLine: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  deviceName: { flexShrink: 1 },
  empty: { paddingVertical: space.space16 },
  revokePanel: { marginTop: space.space16, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 },
  actions: { flexDirection: "row", gap: space.space8 },
  action: { flex: 1 },
  flushBanner: { marginHorizontal: 0, marginTop: space.space20 },
});
