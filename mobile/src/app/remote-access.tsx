// Remote Access: who can reach this host and how. Pair a device with a rotating QR (full access
// or view only), pick the address it should use, and revoke one device without touching the
// others. Direct daemon connections only, and owner-only on the host.
import * as Clipboard from "expo-clipboard";
import { Copy, Radio, Smartphone } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { QrCode } from "../components/QrCode";
import { PageHeader } from "../components/ds/PageHeader";
import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { ConfirmDialog } from "../components/ds/ConfirmDialog";
import { EmptyState } from "../components/ds/EmptyState";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { StatusDot } from "../components/ds/StatusDot";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import type { AccessPreset, AddressKind, PairedClient, PairResult } from "../lib/apiAccess";
import { useConnectionAccess, useDirectAccessAvailable, usePairDevice, useRemoteAccess, useRevokeClient } from "../lib/queriesAccess";
import {
  ADDRESS_LABELS,
  defaultAddressKind,
  lastConnectedLabel,
  pairingLink,
  rotateAfterSeconds,
  scopeSummary,
} from "../lib/remoteAccessModel";
import { SettingsShell } from "./(tabs)/settings";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type as typeScale } from "../theme/typography";

const PRESETS = [
  { value: "full" as const, label: "Full access" },
  { value: "viewer" as const, label: "View only" },
];

const MODE_LABELS: Record<string, string> = {
  local: "Local only",
  lan: "LAN, HTTPS",
  anywhere: "Public tunnel",
};

function RemoteAccessBody() {
  const tokens = useTokens();
  const toast = useToast();
  const toastRef = useRef(toast);
  const direct = useDirectAccessAvailable();
  const access = useConnectionAccess();
  const info = useRemoteAccess(direct && access.owner);
  const pair = usePairDevice();
  const pairRef = useRef(pair.mutateAsync);
  useEffect(() => {
    toastRef.current = toast;
    pairRef.current = pair.mutateAsync;
  });
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  const revoke = useRevokeClient();

  const [preset, setPreset] = useState<AccessPreset>("full");
  const [kind, setKind] = useState<AddressKind | null>(null);
  const [pairing, setPairing] = useState(false);
  const [code, setCode] = useState<PairResult | null>(null);
  const [target, setTarget] = useState<PairedClient | null>(null);

  const addresses = info.data?.addresses ?? [];
  const activeKind = kind ?? defaultAddressKind(addresses, info.data?.mode ?? "local");
  const address = addresses.find((a) => a.kind === activeKind);
  const link = pairingLink(address, code?.token ?? null);

  useEffect(() => {
    if (!pairing) {
      setCode(null);
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const mint = async () => {
      try {
        const result = await pairRef.current({ preset, ttl_secs: 600 });
        if (cancelled) return;
        setCode(result);
        timer = setTimeout(() => void mint(), rotateAfterSeconds(result.expires_in) * 1000);
      } catch (reason) {
        if (cancelled) return;
        setPairing(false);
        toastRef.current.show(reason instanceof Error ? reason.message : "Could not create a pairing code.", { tone: "danger" });
      }
    };
    void mint();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [pairing, preset]);

  const copy = useCallback(
    (text: string, what: string) => {
      void Clipboard.setStringAsync(text).then(() => toast.show(`${what} copied.`, { tone: "neutral" }));
    },
    [toast],
  );

  const confirmRevoke = useCallback(async () => {
    if (!target) return;
    try {
      await revoke.mutateAsync(target.id);
      toast.show(`${target.label} was revoked.`, { tone: "neutral" });
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Could not revoke the device.", { tone: "danger" });
    } finally {
      setTarget(null);
    }
  }, [revoke, target, toast]);

  const online = direct && access.owner && info.data?.online === true;
  const clients = info.data?.clients ?? [];

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <PageHeader title="Remote access" subtitle="Pair phones and other controllers with this host." />

      {!direct ? (
        <EmptyState icon={Radio} message="Remote access is managed on the host. Connect to it directly to pair or revoke devices." />
      ) : !access.owner ? (
        <Banner tone="neutral" message="This connection can view but not change. Only the owner connection manages devices." />
      ) : (
        <>
          <View style={styles.statusRow} accessibilityRole="text">
            <StatusDot state={online ? "busy" : "idle"} accessibilityLabel={online ? "online" : "off"} />
            <Text style={[typeScale.body, { color: tokens.ink }]}>{online ? "Online" : info.isPending ? "Checking" : "Off"}</Text>
            {online ? (
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{MODE_LABELS[info.data?.mode ?? ""] ?? info.data?.mode}</Text>
            ) : null}
          </View>

          <SectionHeader>Pair a device</SectionHeader>
          <View style={[styles.panel, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
            <Segmented options={PRESETS} value={preset} onChange={setPreset} testID="pair-preset" />
            <Segmented
              options={(["local", "lan", "tailscale", "public"] as const).map((k) => ({ value: k, label: ADDRESS_LABELS[k] }))}
              value={activeKind}
              onChange={setKind}
              testID="pair-address"
            />
            {address?.base ? (
              <View style={styles.addressLine}>
                <Text selectable numberOfLines={1} style={[styles.mono, { color: tokens.ink }]}>
                  {address.base}
                </Text>
                <Button
                  label="Copy address"
                  variant="ghost"
                  icon={<Copy size={14} color={tokens.ink2} />}
                  onPress={() => copy(address.base as string, "Address")}
                />
              </View>
            ) : null}
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{address?.note ?? ""}</Text>

            {pairing && link ? (
              <View style={styles.codeBlock}>
                <QrCode value={link} size={176} accessibilityLabel="Pairing QR code" />
                <View style={styles.codeCopy}>
                  <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
                    Scan with the Forge app, or paste the link. It works once and a new code replaces it every few minutes.
                  </Text>
                  <Text selectable numberOfLines={2} style={[styles.mono, { color: tokens.ink3 }]}>
                    {link}
                  </Text>
                  <View style={styles.row}>
                    <Button label="Copy link" variant="secondary" onPress={() => copy(link, "Link")} />
                    <Button label="Done" variant="ghost" onPress={() => setPairing(false)} />
                  </View>
                </View>
              </View>
            ) : pairing && address?.base == null ? (
              <Banner tone="warn" message="This address is not reachable right now. Pick another tab." />
            ) : (
              <Button
                label="Show pairing code"
                variant="primary"
                disabled={address?.base == null}
                loading={pairing}
                onPress={() => setPairing(true)}
              />
            )}
          </View>

          <SectionHeader>Paired devices</SectionHeader>
          {clients.length === 0 ? (
            <EmptyState icon={Smartphone} message="No paired devices yet." />
          ) : (
            <View style={[styles.list, { borderTopColor: tokens.border }]}>
              {clients.map((client, i) => (
                <ListRow
                  key={client.id}
                  title={client.label}
                  subtitle={`${lastConnectedLabel(client, now)} - ${scopeSummary(client)}`}
                  leading={<Smartphone size={18} color={tokens.ink3} />}
                  trailing={
                    <Button
                      label="Revoke"
                      variant="ghost"
                      accessibilityLabel={`Revoke ${client.label}`}
                      onPress={() => setTarget(client)}
                    />
                  }
                  hasInteractiveTrailing
                  showSeparator={i < clients.length - 1}
                />
              ))}
            </View>
          )}
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            Revoking cuts one device off at once. The owner link and every other device keep working.
          </Text>
        </>
      )}

      <ConfirmDialog
        visible={target != null}
        title={`Revoke ${target?.label ?? "device"}?`}
        message="It disconnects immediately and must be paired again to come back."
        confirmLabel="Revoke"
        destructive
        loading={revoke.isPending}
        onConfirm={() => void confirmRevoke()}
        onCancel={() => setTarget(null)}
      />
    </Screen>
  );
}

export default function RemoteAccessScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="remote-access">
        <RemoteAccessBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  statusRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  panel: { borderWidth: 1, borderRadius: radii.radius8, padding: space.space16, gap: space.space12 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, flexWrap: "wrap" },
  addressLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8 },
  codeBlock: { flexDirection: "row", gap: space.space16, flexWrap: "wrap", alignItems: "flex-start" },
  codeCopy: { flex: 1, minWidth: 200, gap: space.space8 },
  mono: { fontFamily: monoFamily.regular, fontSize: 12, flexShrink: 1 },
  list: { borderTopWidth: 1 },
});
