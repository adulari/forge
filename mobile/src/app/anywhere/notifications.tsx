import { Bell, LockKeyhole, RefreshCw } from "lucide-react-native";
import * as Linking from "expo-linking";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereNotificationsScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const [saving, setSaving] = useState(false);
  const subscribed = anywhere.pushStatus === "subscribed";
  const unsupported = anywhere.pushStatus === "unsupported";

  const toggle = async () => {
    setSaving(true);
    try {
      if (subscribed) await anywhere.disablePush();
      else await anywhere.enablePush();
    } catch {
      // The provider exposes a stable, content-free service error below.
    } finally {
      setSaving(false);
    }
  };

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Anywhere" />
      <View style={styles.heading}>
        <Bell size={24} color={tokens.accent} />
        <View style={styles.grow}>
          <Text style={[type.title, { color: tokens.ink }]}>Generic notifications</Text>
          <Text style={[type.sub, { color: tokens.ink2 }]}>Know when Forge needs you without putting workspace details on the lock screen.</Text>
        </View>
      </View>
      {anywhere.error ? <Banner tone="danger" message={anywhere.error} /> : null}
      {anywhere.pushStatus === "denied" ? (
        <Banner tone="warn" message="Notifications are blocked. Allow Forge in iOS Settings, then return here." />
      ) : null}
      <Card style={styles.cardGap}>
        <View style={styles.heading}>
          <LockKeyhole size={20} color={tokens.ink2} />
          <Text style={[type.heading, styles.grow, { color: tokens.ink }]}>Content stays inside Forge</Text>
        </View>
        <Text style={[type.sub, { color: tokens.ink2 }]}>Every alert says only “Open Forge to view an update.” Opening or receiving one asks the signed-in app to refresh; the notification carries no prompt, command, filename, repository, diff, or transcript text.</Text>
      </Card>
      <Card padded={false}>
        <KeyValueRow label="Status" value={statusLabel(anywhere.pushStatus)} />
        <KeyValueRow label="Provider" value="Apple Push Notification service" />
        <KeyValueRow label="Service data" value="Encrypted token + generic event category" />
      </Card>
      <Button
        label={anywhere.pushStatus === "denied" ? "Open iOS Settings" : subscribed ? "Turn off notifications" : "Enable generic notifications"}
        variant={subscribed ? "secondary" : "primary"}
        icon={<RefreshCw size={18} color={subscribed ? tokens.ink : tokens.onAccent} />}
        loading={saving}
        disabled={saving || unsupported}
        onPress={() => {
          if (anywhere.pushStatus === "denied") void Linking.openSettings();
          else void toggle();
        }}
        fullWidth
      />
      <Text style={[type.meta, { color: tokens.ink3 }]}>Forge Anywhere calls the existing APNs-only Forge relay. That relay remains isolated from session relay, sync, and billing infrastructure.</Text>
    </Screen>
  );
}

function statusLabel(status: "unsupported" | "denied" | "unsubscribed" | "subscribed"): string {
  if (status === "subscribed") return "Enabled";
  if (status === "denied") return "Blocked in iOS Settings";
  if (status === "unsupported") return "iPhone only";
  return "Off";
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  heading: { flexDirection: "row", alignItems: "flex-start", gap: space.space12 },
  grow: { flex: 1 },
  cardGap: { gap: space.space12 },
});
