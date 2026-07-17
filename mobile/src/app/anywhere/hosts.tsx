import { Laptop, Trash2 } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Card } from "../../components/ds/Card";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { EmptyState } from "../../components/ds/EmptyState";
import { IconButton } from "../../components/ds/IconButton";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { useAuth } from "../../lib/auth";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereHostsScreen() {
  const anywhere = useAnywhere();
  const auth = useAuth();
  const tokens = useTokens();
  const [targetId, setTargetId] = useState<string | null>(null);
  const [revoking, setRevoking] = useState(false);
  const target = anywhere.hosts.find((host) => host.id === targetId) ?? null;

  const revoke = async () => {
    if (!target) return;
    const host = target;
    setTargetId(null);
    setRevoking(true);
    try { await anywhere.revokeHost(host.id); }
    catch { /* Provider exposes the safe service error. */ }
    finally { setRevoking(false); }
  };

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Anywhere" />
      <Text style={[type.title, { color: tokens.ink }]}>Your hosts</Text>
      <Text style={[type.sub, { color: tokens.ink2 }]}>Choose which enrolled machine this app controls. Cached names and presence remain visible offline; direct and LAN servers stay in the regular server list.</Text>
      {anywhere.error ? <Banner tone="danger" message={anywhere.error} /> : null}
      {anywhere.hosts.length === 0 ? (
        <EmptyState icon={Laptop} message="No hosts connected. Run `forge anywhere enable` on a computer to add it here. Your trial starts when the first host connects." />
      ) : (
        <Card padded={false}>
          {anywhere.hosts.map((host, index) => {
            const selected = auth.activeServerId === `anywhere:${host.id}`;
            const online = isOnline(host.last_heartbeat_at);
            return (
              <ListRow
                key={host.id}
                title={host.name}
                subtitle={online ? "Online now" : lastSeen(host.last_heartbeat_at)}
                leading={<Laptop size={20} color={online ? tokens.success : tokens.ink3} />}
                trailing={<View style={styles.trailing}>
                  {selected ? <Badge label="active" tone="accent" /> : null}
                  <IconButton icon={<Trash2 size={20} color={tokens.danger} />} accessibilityLabel={`Revoke ${host.name}`} onPress={() => setTargetId(host.id)} disabled={revoking} />
                </View>}
                onPress={() => anywhere.selectHost(host.id)}
                hasInteractiveTrailing
                showSeparator={index !== anywhere.hosts.length - 1}
              />
            );
          })}
        </Card>
      )}
      <Text style={[type.meta, { color: tokens.ink3 }]}>Up to three active hosts are included. The service supports host revocation; rename remains available by re-enabling the connector with the desired host name.</Text>
      <ConfirmDialog
        visible={target != null}
        title={`Revoke ${target?.name ?? "host"}?`}
        message="The connector will lose managed relay access. Local Forge, LAN access, and user-managed tunnels on that machine remain unchanged."
        confirmLabel={revoking ? "Revoking…" : "Revoke host"}
        destructive
        onConfirm={() => void revoke()}
        onCancel={() => { if (!revoking) setTargetId(null); }}
      />
    </Screen>
  );
}

function seconds(value: string | null): number | null { const parsed = value ? Number(value) : NaN; return Number.isFinite(parsed) ? parsed : null; }
function isOnline(value: string | null): boolean { const timestamp = seconds(value); return timestamp !== null && Date.now() / 1000 - timestamp < 90; }
function lastSeen(value: string | null): string { const timestamp = seconds(value); return timestamp === null ? "Never connected" : `Last seen ${new Date(timestamp * 1000).toLocaleString()}`; }
const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  trailing: { flexDirection: "row", alignItems: "center", gap: space.space4 },
});
