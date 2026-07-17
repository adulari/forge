import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Card } from "../../components/ds/Card";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereStorageScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const used = anywhere.account?.storage_used_bytes ?? 0;
  const limit = anywhere.account?.storage_limit_bytes ?? 5 * 1024 ** 3;
  const ratio = Math.min(1, limit > 0 ? used / limit : 0);
  return <Screen scroll contentContainerStyle={styles.content}>
    <BackLink label="Anywhere" />
    <Text style={[type.title, { color: tokens.ink }]}>Encrypted storage</Text>
    <Text style={[type.sub, { color: tokens.ink2 }]}>History and sync objects are encrypted on your device before upload. Storage cannot read their contents.</Text>
    <Card style={styles.cardGap}>
      <View style={[styles.track, { backgroundColor: tokens.bg3 }]} accessibilityRole="progressbar" accessibilityValue={{ min: 0, max: limit, now: used }}>
        <View style={[styles.fill, { backgroundColor: ratio > 0.9 ? tokens.warn : tokens.accent, width: `${ratio * 100}%` }]} />
      </View>
      <Text style={[type.heading, { color: tokens.ink }]}>{format(used)} of {format(limit)} used</Text>
      <Text style={[type.sub, { color: tokens.ink2 }]}>{Math.round(ratio * 100)}% of your personal storage allowance</Text>
    </Card>
    <Card padded={false}>
      <KeyValueRow label="Quota" value="5 GB" />
      <KeyValueRow label="Encryption" value={`Account key epoch ${anywhere.credentials?.keyEpoch ?? "—"}`} />
      <KeyValueRow label="Over quota" value="Downloads and deletion stay available" />
    </Card>
    <Text style={[type.meta, { color: tokens.ink3 }]}>Storage includes encrypted sync records, temporary relay blobs, capsules, and replay shares according to their retention windows.</Text>
  </Screen>;
}
function format(bytes: number): string { return bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(2)} GB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`; }
const styles = StyleSheet.create({ content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 }, cardGap: { gap: space.space8 }, track: { height: 10, borderRadius: radii.radiusPill, overflow: "hidden" }, fill: { height: "100%", borderRadius: radii.radiusPill } });
