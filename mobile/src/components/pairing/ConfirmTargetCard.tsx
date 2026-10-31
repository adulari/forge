import { ShieldAlert, ShieldCheck } from "lucide-react-native";
import React from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import type { ConnectTarget } from "../../lib/connectUrl";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";

export interface ConfirmTargetCardProps {
  target: ConnectTarget;
  busy?: boolean;
  onConnect: () => void;
  onCancel: () => void;
}

export function ConfirmTargetCard({ target, busy = false, onConnect, onCancel }: ConfirmTargetCardProps) {
  const tokens = useTokens();
  const risky = target.cleartext || target.lookalike || target.pinIgnored;
  const Icon = risky ? ShieldAlert : ShieldCheck;
  return (
    <View
      style={[styles.card, { borderColor: risky ? tokens.warn : tokens.border, backgroundColor: tokens.bg2, borderRadius: radii.radius8 }]}
      accessibilityRole="summary"
    >
      <Text style={[typeScale.section, { color: tokens.ink4 }]}>connect to</Text>
      <View style={styles.hostRow}>
        <Icon size={18} strokeWidth={1.75} color={risky ? tokens.warn : tokens.success} />
        <Text style={[typeScale.code, styles.host, { color: tokens.ink }]} selectable numberOfLines={2}>
          {target.host}
        </Text>
      </View>
      {target.pinned ? (
        <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
          this code pins the server&apos;s certificate, so a different machine on this network can&apos;t take its place.
        </Text>
      ) : null}
      {target.pinIgnored ? (
        <Text style={[typeScale.sub, { color: tokens.warn }]}>
          this link carries a certificate pin, but this device does not check it, so it does not protect the
          connection. only continue if you trust the network and the address.
        </Text>
      ) : null}
      {target.cleartext ? (
        <Text style={[typeScale.sub, { color: tokens.warn }]}>
          this address uses plain http. anyone on the network path can read your token and everything you send.
          {target.private ? " it looks like a private network address." : " it is not a private network address."}
        </Text>
      ) : null}
      {target.lookalike ? (
        <Text style={[typeScale.sub, { color: tokens.warn }]}>
          this host name uses look-alike characters. check it against what your computer shows before you go on.
        </Text>
      ) : null}
      {Platform.OS === "android" && target.private ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          if android asks to find devices on your local network, allow it so forge can reach this computer.
        </Text>
      ) : null}
      <View style={styles.actions}>
        <Button
          label={target.cleartext ? "Connect anyway" : "Connect"}
          variant={target.cleartext ? "secondary" : "primary"}
          onPress={onConnect}
          loading={busy}
          fullWidth
        />
        <Button label="Cancel" variant="ghost" onPress={onCancel} disabled={busy} fullWidth />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, padding: space.space16, gap: space.space12, width: "100%" },
  hostRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  host: { flex: 1 },
  actions: { gap: space.space8, marginTop: space.space4 },
});
