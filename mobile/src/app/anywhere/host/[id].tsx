import { Redirect, router, useLocalSearchParams } from "expo-router";
import { Laptop, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../../components/ds/BackLink";
import { Button } from "../../../components/ds/Button";
import { Input } from "../../../components/ds/Input";
import { Screen } from "../../../components/ds/Screen";
import { useToast } from "../../../components/ds/ToastHost";
import { useAnywhere } from "../../../lib/AnywhereProvider";
import { useTokens } from "../../../theme/ThemeProvider";
import { radii, space } from "../../../theme/tokens";
import { type as typeScale } from "../../../theme/typography";

export default function AnywhereHostScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const host = anywhere.hosts.find((candidate) => candidate.id === id);
  const [name, setName] = useState(host?.name ?? "");
  const [busy, setBusy] = useState(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);

  useEffect(() => { if (host) setName(host.name); }, [host]);

  const rename = useCallback(async () => {
    if (!host || name.trim() === host.name) return;
    setBusy(true);
    try { await anywhere.renameHost(host.id, name); toast.show("Host renamed.", { tone: "neutral" }); }
    catch (reason) { toast.show(reason instanceof Error ? reason.message : "Host could not be renamed.", { tone: "danger" }); }
    finally { setBusy(false); }
  }, [anywhere, host, name, toast]);

  const revoke = useCallback(async () => {
    if (!host) return;
    setBusy(true);
    try { await anywhere.revokeHost(host.id); toast.show("Host revoked. Local Forge data was not deleted.", { tone: "neutral" }); router.replace("/anywhere"); }
    catch (reason) { toast.show(reason instanceof Error ? reason.message : "Host could not be revoked.", { tone: "danger" }); }
    finally { setBusy(false); }
  }, [anywhere, host, toast]);

  if (anywhere.phase !== "ready") return <Redirect href="/anywhere" />;
  if (!host) return <Redirect href="/anywhere" />;

  return <Screen scroll keyboardAvoiding contentContainerStyle={styles.screen}><View style={styles.shell}>
    <BackLink label="Forge Anywhere" />
    <View style={styles.header}><View style={[styles.icon, { backgroundColor: tokens.selection }]}><Laptop size={22} color={tokens.accent} /></View><View style={styles.headerCopy}><Text accessibilityRole="header" style={[typeScale.title, { color: tokens.ink }]}>{host.name}</Text><Text style={[typeScale.sub, { color: tokens.ink3 }]}>{host.last_heartbeat_at ? `Last active ${new Date(host.last_heartbeat_at).toLocaleString()}` : "Waiting for first connection"}</Text></View></View>
    <View style={styles.form}><Input label="Host name" value={name} onChangeText={setName} maxLength={80} /><Button label="Save host name" onPress={() => void rename()} loading={busy} disabled={!name.trim() || name.trim() === host.name} fullWidth /></View>
    <View style={[styles.danger, { borderColor: tokens.borderStrong }]}><Text style={[typeScale.headingBold, { color: tokens.ink }]}>Remove managed access</Text><Text style={[typeScale.sub, { color: tokens.ink2 }]}>Revoking disconnects this host from Forge Anywhere. Projects and other local Forge data stay on the computer.</Text>{confirmRevoke ? <View style={styles.actions}><Button label="Keep host" variant="ghost" disabled={busy} onPress={() => setConfirmRevoke(false)} style={styles.action} /><Button label="Revoke host" variant="danger" icon={<Trash2 size={17} color={tokens.danger} />} loading={busy} onPress={() => void revoke()} style={styles.action} /></View> : <Button label="Revoke host" variant="danger" onPress={() => setConfirmRevoke(true)} fullWidth />}</View>
  </View></Screen>;
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, paddingBottom: space.space48 }, shell: { width: "100%", maxWidth: 680, alignSelf: "center" },
  header: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space12 }, icon: { width: 44, height: 44, borderRadius: radii.radius12, alignItems: "center", justifyContent: "center" }, headerCopy: { flex: 1 },
  form: { marginTop: space.space24, gap: space.space12 }, danger: { marginTop: space.space32, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 }, actions: { flexDirection: "row", gap: space.space8 }, action: { flex: 1 },
});
