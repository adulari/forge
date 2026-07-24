// Forge Anywhere — Host detail (desktop.dc.html "D AW Host Detail" lines 1028-1040,
// mobile.dc.html "M AW Hosts Detail Pair" lines 508-514).
//
// Every row here states something this device can actually verify:
//   · Identity — SHA256 of the host device's enrolled signing key, so it genuinely does
//     not change when the host is renamed. Hidden until that key is known.
//   · Connector — version only when the service returns `connector_version` (it does not
//     yet); the heartbeat age is always real.
//   · Reachable via — the transports this device holds: the registered relay, plus a
//     saved LAN target when one reports the same daemon hostname.
//   · Transport for new sessions — a device-local preference `selectHost` honours.
//   · Disable — offered only when the service reports a `disabled` field for the host.
import { Redirect, router, useLocalSearchParams } from "expo-router";
import { Laptop, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../../components/ds/BackLink";
import { Button } from "../../../components/ds/Button";
import { Input } from "../../../components/ds/Input";
import { Screen } from "../../../components/ds/Screen";
import { Segmented } from "../../../components/ds/Segmented";
import { useToast } from "../../../components/ds/ToastHost";
import { useAnywhere } from "../../../lib/AnywhereProvider";
import { useAuth } from "../../../lib/auth";
import { directServerForHost, formatReachability, hostIdentityFingerprint, hostReachability } from "../../../lib/anywhere/hostIdentity";
import type { TransportPreference } from "../../../lib/anywhere/types";
import { hostLastActiveMs, hostStatusText } from "../../../lib/anywhereHostPresence";
import { useTokens } from "../../../theme/ThemeProvider";
import { useBreakpoint } from "../../../theme/useBreakpoint";
import { radii, space } from "../../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../../theme/typography";

const TRANSPORT_OPTIONS: { value: TransportPreference; label: string }[] = [
  { value: "auto", label: "AUTO" },
  { value: "direct", label: "DIRECT" },
  { value: "anywhere", label: "ANYWHERE" },
];

function heartbeatLabel(lastActiveMs: number | null): string | null {
  if (lastActiveMs === null) return null;
  const deltaSec = Math.max(0, Math.round((Date.now() - lastActiveMs) / 1000));
  if (deltaSec < 60) return `heartbeat ${deltaSec}s`;
  const deltaMin = Math.round(deltaSec / 60);
  if (deltaMin < 60) return `heartbeat ${deltaMin}m`;
  const deltaHour = Math.round(deltaMin / 60);
  return `heartbeat ${deltaHour}h`;
}

function transportCaption(preference: TransportPreference, directName: string | null): string {
  switch (preference) {
    case "direct":
      return directName
        ? `direct · opens the saved target "${directName}"`
        : "direct · no saved LAN target for this hostname — the relay is used";
    case "anywhere":
      return "anywhere · always the managed encrypted relay";
    case "auto":
    default:
      return directName
        ? "auto · managed relay · pick DIRECT to use the saved LAN target"
        : "auto · managed relay";
  }
}

function IdentityRow({ label, value }: { label: string; value: string }) {
  const tokens = useTokens();
  // The desktop comp's identity block is a dense 32px stack; touch targets only apply
  // where a row is tappable, and these rows are read-only.
  const { isExpanded } = useBreakpoint();
  return (
    <View style={[styles.identityRow, { minHeight: isExpanded ? 32 : 44 }]}>
      <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{label}</Text>
      <Text style={[typeScale.monoMeta, tabularNums, styles.identityValue, { color: tokens.ink2 }]}>{value}</Text>
    </View>
  );
}

export default function AnywhereHostScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const anywhere = useAnywhere();
  const { servers } = useAuth();
  const tokens = useTokens();
  const toast = useToast();
  const host = anywhere.hosts.find((candidate) => candidate.id === id);
  const [name, setName] = useState(host?.name ?? "");
  const [busy, setBusy] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);

  useEffect(() => { if (host) setName(host.name); }, [host]);

  const fingerprint = useMemo(
    () => hostIdentityFingerprint(host ? anywhere.credentials?.signingPublicKeys[host.device_id] : undefined),
    [anywhere.credentials?.signingPublicKeys, host],
  );
  const directTarget = useMemo(
    () => (host ? directServerForHost(servers, host.name) : null),
    [host, servers],
  );
  const reachable = useMemo(
    () => (host ? formatReachability(hostReachability(servers, host.name)) : ""),
    [host, servers],
  );

  const rename = useCallback(async () => {
    if (!host || name.trim() === host.name) return;
    setBusy(true);
    try { await anywhere.renameHost(host.id, name); toast.show("Host renamed. Its identity fingerprint is unchanged.", { tone: "neutral" }); }
    catch (reason) { toast.show(reason instanceof Error ? reason.message : "Host could not be renamed.", { tone: "danger" }); }
    finally { setBusy(false); }
  }, [anywhere, host, name, toast]);

  const changeTransport = useCallback(async (preference: TransportPreference) => {
    if (!host) return;
    try { await anywhere.setHostTransportPreference(host.id, preference); }
    catch (reason) { toast.show(reason instanceof Error ? reason.message : "Preference could not be saved.", { tone: "danger" }); }
  }, [anywhere, host, toast]);

  const toggleDisabled = useCallback(async () => {
    if (!host || host.disabled === undefined) return;
    const next = !host.disabled;
    setBusy(true);
    try {
      await anywhere.setHostDisabled(host.id, next);
      toast.show(next ? "Host disabled. It keeps its enrollment and identity." : "Host enabled.", { tone: "neutral" });
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Host could not be updated.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, host, toast]);

  const revoke = useCallback(async () => {
    if (!host) return;
    setBusy(true);
    try { await anywhere.revokeHost(host.id); toast.show("Host revoked. Local Forge data was not deleted.", { tone: "neutral" }); router.replace("/anywhere"); }
    catch (reason) { toast.show(reason instanceof Error ? reason.message : "Host could not be revoked.", { tone: "danger" }); }
    finally { setBusy(false); }
  }, [anywhere, host, toast]);

  if (anywhere.phase !== "ready") return <Redirect href="/anywhere" />;
  if (!host) return <Redirect href="/anywhere" />;

  const heartbeat = heartbeatLabel(hostLastActiveMs(host));
  const connector = [host.connector_version ? `v${host.connector_version}` : null, heartbeat ?? "no heartbeat yet"]
    .filter(Boolean)
    .join(" · ");
  const preference = anywhere.hostTransportPreferences[host.id] ?? "auto";

  return <Screen scroll keyboardAvoiding contentContainerStyle={styles.screen}><View style={styles.shell}>
    <BackLink label="Forge Anywhere" />
    <View style={styles.header}>
      <View style={[styles.icon, { backgroundColor: tokens.selection }]}><Laptop size={22} color={tokens.accent} /></View>
      <View style={styles.headerCopy}>
        <Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>{host.name}</Text>
        <Text style={[typeScale.monoMeta, tabularNums, { color: host.online === true ? tokens.success : tokens.ink3 }]}>
          {host.disabled === true ? "disabled" : hostStatusText(host)}
        </Text>
      </View>
    </View>

    <View style={[styles.identity, { borderTopColor: tokens.border }]}>
      {fingerprint ? <IdentityRow label="Identity" value={`${fingerprint} · unchanged by rename`} /> : null}
      <IdentityRow label="Connector" value={connector} />
      <IdentityRow label="Reachable via" value={reachable} />
    </View>

    <View style={styles.transport}>
      <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Transport for new sessions</Text>
      <Segmented options={TRANSPORT_OPTIONS} value={preference} onChange={(value) => void changeTransport(value)} testID="host-transport-segmented" />
      <Text style={[typeScale.monoMeta, styles.transportCaption, { color: tokens.ink4 }]}>
        {`${transportCaption(preference, directTarget?.name ?? null)} · stored on this device`}
      </Text>
    </View>

    <View style={styles.form}>
      <Input label="Host name" value={name} onChangeText={setName} maxLength={80} />
      <Button label="Save host name" onPress={() => void rename()} loading={busy} disabled={!name.trim() || name.trim() === host.name} fullWidth />
    </View>

    <View style={[styles.danger, { borderColor: tokens.borderStrong }]}>
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Remove managed access</Text>
      <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
        Revoking disconnects this host from Forge Anywhere. Projects and other local Forge data stay on the computer.
      </Text>
      {host.disabled !== undefined ? (
        <Button
          label={host.disabled ? "Enable host" : "Disable host"}
          variant="secondary"
          loading={busy}
          onPress={() => void toggleDisabled()}
          fullWidth
        />
      ) : null}
      {confirmRevoke ? (
        <View style={styles.actions}>
          <Button label="Keep host" variant="ghost" disabled={busy} onPress={() => setConfirmRevoke(false)} style={styles.action} />
          <Button label="Revoke host" variant="danger" icon={<Trash2 size={17} color={tokens.danger} />} loading={busy} onPress={() => void revoke()} style={styles.action} />
        </View>
      ) : (
        <Button label="Revoke host" variant="danger" onPress={() => setConfirmRevoke(true)} fullWidth />
      )}
    </View>
  </View></Screen>;
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, paddingBottom: space.space48 }, shell: { width: "100%", maxWidth: 680, alignSelf: "center" },
  header: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space12 }, icon: { width: 44, height: 44, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" }, headerCopy: { flex: 1, gap: 2 },
  identity: { marginTop: space.space16, borderTopWidth: 1, paddingTop: space.space8 },
  identityRow: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: space.space12 },
  identityValue: { flexShrink: 1, textAlign: "right" },
  transport: { marginTop: space.space16, gap: space.space8 },
  transportCaption: { lineHeight: 16, fontFamily: monoFamily.regular },
  form: { marginTop: space.space24, gap: space.space12 }, danger: { marginTop: space.space32, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 }, actions: { flexDirection: "row", gap: space.space8 }, action: { flex: 1 },
});
