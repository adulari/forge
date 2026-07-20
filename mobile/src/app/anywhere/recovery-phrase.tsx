import * as DocumentPicker from "expo-document-picker";
import { File as ExpoFile } from "expo-file-system";
import { Redirect } from "expo-router";
import { Check, KeyRound, Laptop, Pencil, Plus, ShieldAlert, ShieldCheck, Smartphone, Trash2 } from "lucide-react-native";
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

export default function RecoveryCenterScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const toast = useToast();
  const [target, setTarget] = useState<string | null>(null);
  const [kit, setKit] = useState("");
  const [busy, setBusy] = useState(false);
  const [passkeyKit, setPasskeyKit] = useState("");
  const [passkeyTarget, setPasskeyTarget] = useState<string | null>(null);
  const [passkeyName, setPasskeyName] = useState("");

  const importKit = useCallback(async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: ["application/json", "application/octet-stream", "text/plain"], multiple: false, copyToCacheDirectory: true });
    if (result.canceled) return;
    try {
      setKit(await new ExpoFile(result.assets[0].uri).text());
      toast.show("Recovery Kit loaded for this revocation.", { tone: "neutral" });
    } catch {
      toast.show("That Recovery Kit could not be read.", { tone: "danger" });
    }
  }, [toast]);

  const importPasskeyKit = useCallback(async () => {
    const result = await DocumentPicker.getDocumentAsync({ type: ["application/json", "application/octet-stream", "text/plain"], multiple: false, copyToCacheDirectory: true });
    if (result.canceled) return;
    try {
      setPasskeyKit(await new ExpoFile(result.assets[0].uri).text());
      toast.show("Recovery Kit loaded for passkey registration.", { tone: "neutral" });
    } catch {
      toast.show("That Recovery Kit could not be read.", { tone: "danger" });
    }
  }, [toast]);

  const registerPasskey = useCallback(async () => {
    if (!passkeyKit.trim()) return;
    setBusy(true);
    try {
      await anywhere.registerPasskey(passkeyKit);
      setPasskeyKit("");
      toast.show("Recovery passkey registered.", { tone: "neutral" });
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Passkey registration failed.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, passkeyKit, toast]);

  const revoke = useCallback(async () => {
    if (!target || !kit.trim()) return;
    setBusy(true);
    try {
      await anywhere.revokeDevice(target, kit);
      toast.show("Device revoked and account keys rotated.", { tone: "neutral" });
      setTarget(null);
      setKit("");
    } catch (reason) {
      toast.show(reason instanceof Error ? reason.message : "Device could not be revoked.", { tone: "danger" });
    } finally { setBusy(false); }
  }, [anywhere, kit, target, toast]);

  if (anywhere.phase !== "ready") return <Redirect href="/anywhere" />;

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.screen}>
      <View style={styles.shell}>
        <BackLink label="Forge Anywhere" />
        <Text accessibilityRole="header" style={[typeScale.title, styles.title, { color: tokens.ink }]}>Recovery Center</Text>
        <Text style={[typeScale.body, styles.subtitle, { color: tokens.ink2 }]}>Check how you can regain access and remove devices without making recovery words part of normal sign-in.</Text>

        <View style={styles.health}>
          <HealthRow icon={anywhere.credentials?.recoveryKitVerified ? <ShieldCheck size={19} color={tokens.success} /> : <ShieldAlert size={19} color={tokens.warn} />} title="Recovery Kit" value={anywhere.credentials?.recoveryKitVerified ? "Verified on this device" : "Not checked on this device"} />
          <HealthRow icon={<KeyRound size={19} color={anywhere.passkeys.length ? tokens.success : tokens.warn} />} title="Passkey recovery" value={anywhere.passkeys.length ? `${anywhere.passkeys.length} registered` : "Recommended · none registered"} />
          <HealthRow icon={<Laptop size={19} color={tokens.info} />} title="Enrolled devices" value={`${anywhere.devices.length} available`} />
        </View>

        {!anywhere.credentials?.recoveryKitVerified ? <Banner tone="warn" message="Keep your Recovery Kit offline. This device has not verified it during the current enrollment." style={styles.flushBanner} /> : null}
        {anywhere.account?.pending_reset ? <Banner tone="danger" message={`Clean reset scheduled for ${new Date(anywhere.account.pending_reset.executes_at_ms).toLocaleString()}.`} actionLabel="Cancel reset" onAction={() => void anywhere.cancelCleanReset().then(() => toast.show("Clean reset canceled.", { tone: "neutral" })).catch((reason) => toast.show(reason instanceof Error ? reason.message : "Reset could not be canceled.", { tone: "danger" }))} style={styles.flushBanner} /> : null}

        <View style={styles.section}>
          <View style={styles.sectionHeader}><Text style={[typeScale.headingBold, { color: tokens.ink }]}>Passkeys</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>Optional, recommended</Text></View>
          <View style={[styles.deviceList, { borderTopColor: tokens.border }]}>
            {anywhere.passkeys.map((passkey) => <View key={passkey.id} style={[styles.deviceRow, { borderBottomColor: tokens.hairline }]}><KeyRound size={18} color={tokens.success} /><View style={styles.deviceCopy}><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{passkey.name}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{passkey.last_used_at ? `Last used ${new Date(passkey.last_used_at * 1000).toLocaleString()}` : `Added ${new Date(passkey.created_at * 1000).toLocaleDateString()}`}</Text></View><Pressable accessibilityRole="button" accessibilityLabel={`Rename ${passkey.name}`} onPress={() => { setPasskeyTarget(passkey.id); setPasskeyName(passkey.name); }} style={styles.iconButton}><Pencil size={16} color={tokens.ink2} /></Pressable><Pressable accessibilityRole="button" accessibilityLabel={`Remove ${passkey.name}`} onPress={() => void anywhere.revokePasskey(passkey.id).then(() => toast.show("Passkey removed.", { tone: "neutral" })).catch((reason) => toast.show(reason instanceof Error ? reason.message : "Passkey could not be removed.", { tone: "danger" }))} style={styles.iconButton}><Trash2 size={16} color={tokens.danger} /></Pressable></View>)}
            {!anywhere.passkeys.length ? <View style={[styles.deviceRow, { borderBottomColor: tokens.hairline }]}><KeyRound size={18} color={tokens.ink3} /><Text style={[typeScale.sub, { color: tokens.ink3 }]}>No recovery passkeys registered yet.</Text></View> : null}
          </View>
          {passkeyTarget ? <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}><Text style={[typeScale.headingBold, { color: tokens.ink }]}>Rename passkey</Text><Input label="Passkey name" value={passkeyName} onChangeText={setPasskeyName} /><View style={styles.actions}><Button label="Cancel" variant="ghost" onPress={() => setPasskeyTarget(null)} style={styles.action} /><Button label="Save name" onPress={() => void anywhere.renamePasskey(passkeyTarget, passkeyName).then(() => { setPasskeyTarget(null); toast.show("Passkey renamed.", { tone: "neutral" }); }).catch((reason) => toast.show(reason instanceof Error ? reason.message : "Passkey could not be renamed.", { tone: "danger" }))} disabled={!passkeyName.trim()} style={styles.action} /></View></View> : null}
          <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}><View style={styles.addTitle}><Plus size={18} color={tokens.accent} /><Text style={[typeScale.headingBold, { color: tokens.ink }]}>Add a recovery passkey</Text></View><Text style={[typeScale.sub, { color: tokens.ink2 }]}>Load your Recovery Kit once so this browser can wrap its secret with the new passkey. The secret and PRF output never reach Forge.</Text><Button label="Choose Recovery Kit file" variant="secondary" onPress={() => void importPasskeyKit()} fullWidth /><Input label="Or enter 12 / legacy 24 words" value={passkeyKit.startsWith("{") ? "Recovery Kit file loaded" : passkeyKit} onChangeText={setPasskeyKit} multiline autoCapitalize="none" autoCorrect={false} /><Button label="Create recovery passkey" loading={busy} disabled={!passkeyKit.trim()} onPress={() => void registerPasskey()} fullWidth /></View>
        </View>

        <View style={styles.section}>
          <View style={styles.sectionHeader}><Text style={[typeScale.headingBold, { color: tokens.ink }]}>Devices</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>Revoke lost access</Text></View>
          <View style={[styles.deviceList, { borderTopColor: tokens.border }]}>
            {anywhere.devices.map((device) => {
              const current = device.id === anywhere.credentials?.deviceIdHex;
              return <View key={device.id} style={[styles.deviceRow, { borderBottomColor: tokens.hairline }]}>{device.name.toLowerCase().includes("phone") ? <Smartphone size={18} color={tokens.ink3} /> : <Laptop size={18} color={tokens.ink3} />}<View style={styles.deviceCopy}><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{device.name}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{current ? "This device" : device.last_seen_at ? `Last active ${new Date(device.last_seen_at).toLocaleString()}` : "Enrolled"}</Text></View>{!current ? <Pressable onPress={() => { setTarget(device.id); setKit(""); }} accessibilityRole="button" accessibilityLabel={`Revoke ${device.name}`} style={styles.revokeButton}><Trash2 size={17} color={tokens.danger} /><Text style={[typeScale.bodyBold, { color: tokens.danger }]}>Revoke</Text></Pressable> : <Check size={17} color={tokens.success} />}</View>;
            })}
          </View>
        </View>

        {target ? <View style={[styles.revokePanel, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Confirm device revocation</Text>
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Import your Recovery Kit or enter its 12 words. Forge rotates the account key atomically before the device is removed.</Text>
          <Button label="Choose Recovery Kit file" variant="secondary" onPress={() => void importKit()} fullWidth />
          <Input label="Or enter 12 / legacy 24 words" value={kit.startsWith("{") ? "Recovery Kit file loaded" : kit} onChangeText={setKit} multiline autoCapitalize="none" autoCorrect={false} />
          <View style={styles.actions}><Button label="Cancel" variant="ghost" disabled={busy} onPress={() => { setTarget(null); setKit(""); }} style={styles.action} /><Button label="Revoke device" variant="danger" loading={busy} disabled={!kit.trim()} onPress={() => void revoke()} style={styles.action} /></View>
        </View> : null}

        <View style={styles.accountActions}>
          <Button label="Sign out of Forge Anywhere" variant="ghost" onPress={() => void anywhere.logout()} fullWidth />
        </View>
      </View>
    </Screen>
  );
}

function HealthRow({ icon, title, value }: { icon: React.ReactNode; title: string; value: string }) {
  const tokens = useTokens();
  return <View style={[styles.healthRow, { borderBottomColor: tokens.hairline }]}>{icon}<View style={styles.deviceCopy}><Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{title}</Text><Text style={[typeScale.meta, { color: tokens.ink3 }]}>{value}</Text></View></View>;
}

const styles = StyleSheet.create({
  screen: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 720, alignSelf: "center" },
  title: { marginTop: space.space8 },
  subtitle: { marginTop: space.space4, maxWidth: 620 },
  health: { marginTop: space.space24 },
  healthRow: { minHeight: 58, flexDirection: "row", alignItems: "center", gap: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  section: { marginTop: space.space24, gap: space.space8 },
  sectionHeader: { flexDirection: "row", justifyContent: "space-between", alignItems: "baseline" },
  deviceList: { borderTopWidth: 1 },
  deviceRow: { minHeight: 60, flexDirection: "row", alignItems: "center", gap: space.space12, borderBottomWidth: StyleSheet.hairlineWidth },
  deviceCopy: { flex: 1, gap: 2 },
  revokeButton: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space8 },
  iconButton: { minWidth: 42, minHeight: 42, alignItems: "center", justifyContent: "center" },
  addTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  revokePanel: { marginTop: space.space20, borderWidth: 1, borderRadius: radii.radius12, padding: space.space16, gap: space.space12 },
  actions: { flexDirection: "row", gap: space.space8 },
  action: { flex: 1 },
  flushBanner: { marginHorizontal: 0, marginTop: space.space12 },
  accountActions: { marginTop: space.space32 },
});
