// The Fleet's "Offline hosts" section: paired servers this device cannot reach and Forge
// Anywhere hosts that are offline or stale, so a missing session is explained rather than
// silently absent. Renders nothing when every host is up.
import { router } from "expo-router";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { hostStateText } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { useFleetScope } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { HostDot } from "../anywhere/HostDot";
import { useServerFleetStatuses } from "./ServerScopePills";

export function OfflineHosts() {
  const tokens = useTokens();
  const { selectServer } = useFleetScope();
  const statuses = useServerFleetStatuses();
  const { signedIn } = useAnywhere();
  const { hosts } = useAnywhereHosts();
  const unreachable = statuses.filter((status) => status.reachability === "unreachable");
  const away = signedIn ? hosts.filter((host) => host.state.kind === "offline" || host.state.kind === "stale") : [];
  if (unreachable.length === 0 && away.length === 0) return null;

  return (
    <View style={styles.section}>
      <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>OFFLINE HOSTS</Text>
      {unreachable.map((server) => (
        <Pressable
          key={server.id}
          onPress={() => selectServer(server.id)}
          accessibilityRole="button"
          accessibilityLabel={`${server.name}, unreachable`}
          accessibilityHint="Shows this server and retries it"
          style={[styles.row, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        >
          <View style={[styles.dot, { backgroundColor: tokens.danger }]} />
          <Text style={[typeScale.body, styles.name, { color: tokens.ink2 }]} numberOfLines={1}>
            {server.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>unreachable</Text>
        </Pressable>
      ))}
      {away.map((host) => (
        <Pressable
          key={host.id}
          onPress={() => router.push(`/anywhere/host/${host.id}`)}
          accessibilityRole="button"
          accessibilityLabel={`${host.name}, ${hostStateText(host)}`}
          style={[styles.row, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        >
          <HostDot state={host.state} size={7} />
          <Text style={[typeScale.body, styles.name, { color: tokens.ink2 }]} numberOfLines={1}>
            {host.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{hostStateText(host)}</Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  section: { paddingHorizontal: space.space16, paddingTop: space.space16, gap: space.space8 },
  label: { marginBottom: 2 },
  row: {
    minHeight: 40,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    borderRadius: radii.radius8,
    borderWidth: 1,
  },
  dot: { width: 7, height: 7, borderRadius: 4 },
  name: { flex: 1 },
});
