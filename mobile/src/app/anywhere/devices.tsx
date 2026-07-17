import { router } from "expo-router";
import { MonitorSmartphone, QrCode, ShieldAlert, Smartphone, Trash2 } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { IconButton } from "../../components/ds/IconButton";
import { Input } from "../../components/ds/Input";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereDevicesScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const [targetId, setTargetId] = useState<string | null>(null);
  const [recoveryWords, setRecoveryWords] = useState("");
  const [confirming, setConfirming] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const target = anywhere.devices.find((device) => device.id === targetId) ?? null;
  const phraseComplete = recoveryWords.trim().split(/\s+/).length === 24;

  const revoke = async () => {
    if (!target) return;
    setConfirming(false);
    setRevoking(true);
    try {
      await anywhere.revokeDevice(target.id, recoveryWords);
      setTargetId(null);
    } catch {
      // The provider keeps the active epoch unchanged and exposes the safe service error.
    } finally {
      setRecoveryWords("");
      setRevoking(false);
    }
  };

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <BackLink label="Anywhere" />
      <Text style={[type.title, { color: tokens.ink }]}>Enrolled devices</Text>
      <Text style={[type.sub, { color: tokens.ink2 }]}>Each device has separate signing and exchange keys. Revoking one atomically rotates encrypted data to the remaining devices.</Text>
      {anywhere.error ? <Banner tone="danger" message={anywhere.error} /> : null}
      <Button label="Pair a device with QR" variant="secondary" icon={<QrCode size={18} color={tokens.ink} />} onPress={() => router.push("/anywhere/pair")} fullWidth />
      <Card padded={false}>
        {anywhere.devices.map((device, index) => {
          const current = device.id === anywhere.credentials?.deviceIdHex;
          return (
            <ListRow
              key={device.id}
              title={device.name}
              subtitle={device.last_seen_at ? `Last seen ${new Date(Number(device.last_seen_at) * 1000).toLocaleString()}` : `Added ${new Date(Number(device.created_at) * 1000).toLocaleDateString()}`}
              leading={current ? <Smartphone size={20} color={tokens.accent} /> : <MonitorSmartphone size={20} color={tokens.ink3} />}
              trailing={current ? (
                <Badge label="this device" tone="accent" />
              ) : (
                <IconButton
                  icon={<Trash2 size={20} color={tokens.danger} />}
                  accessibilityLabel={`Revoke ${device.name}`}
                  accessibilityHint="Opens recovery phrase confirmation before rotating keys"
                  onPress={() => { setTargetId(device.id); setRecoveryWords(""); }}
                />
              )}
              hasInteractiveTrailing={!current}
              showSeparator={index !== anywhere.devices.length - 1}
            />
          );
        })}
      </Card>

      {target ? (
        <Card variant="feature" style={styles.rotation}>
          <View style={styles.warningTitle}>
            <ShieldAlert size={22} color={tokens.danger} />
            <Text style={[type.heading, { color: tokens.ink }]}>Revoke {target.name}</Text>
          </View>
          <Text style={[type.sub, { color: tokens.ink2 }]}>Enter your recovery phrase to verify the current account key. It stays in memory only and is cleared after this attempt.</Text>
          <Input
            label="24-word recovery phrase"
            value={recoveryWords}
            onChangeText={setRecoveryWords}
            autoCapitalize="none"
            autoCorrect={false}
            secureTextEntry
            multiline
            numberOfLines={4}
          />
          <View style={styles.actions}>
            <Button label="Cancel" variant="ghost" disabled={revoking} onPress={() => { setTargetId(null); setRecoveryWords(""); }} style={styles.action} />
            <Button label="Review revocation" variant="danger" loading={revoking} disabled={!phraseComplete} onPress={() => setConfirming(true)} style={styles.action} />
          </View>
        </Card>
      ) : null}
      <Text style={[type.meta, { color: tokens.ink3 }]}>The recovery phrase is never stored by this screen. Revocation removes the device’s tokens and hosts only when replacement key wraps commit successfully.</Text>
      <ConfirmDialog
        visible={confirming && target != null}
        title={`Revoke ${target?.name ?? "device"}?`}
        message="This signs the device out, removes its hosts, and rotates the account data key. The action cannot be undone."
        confirmLabel="Revoke and rotate keys"
        destructive
        onConfirm={() => void revoke()}
        onCancel={() => setConfirming(false)}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  rotation: { gap: space.space12 },
  warningTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  actions: { flexDirection: "row", gap: space.space8 },
  action: { flex: 1 },
});
