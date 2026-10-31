// The Fleet's "Offline hosts" section: paired servers this device cannot reach and Forge
// Anywhere hosts that are offline or stale, so a missing session is explained rather than
// silently absent. Renders nothing when every host is up.
import { router } from "expo-router";
import React, { useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { hostStateText } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { useAuth } from "../../lib/auth";
import { lastSeenLabel } from "../../lib/hostPresence";
import { probePin } from "../../lib/pinnedLan";
import { isTauri } from "../../lib/platform";
import { useFleetScope } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { HostDot } from "../anywhere/HostDot";
import { useServerFleetStatuses } from "./ServerScopePills";

function useChangedCertificates(unreachableIds: string[]): Set<string> {
  const { servers } = useAuth();
  const [changed, setChanged] = useState<Set<string>>(new Set());
  const key = unreachableIds.join(",");
  useEffect(() => {
    if (!isTauri || key === "") return;
    let cancelled = false;
    const pinned = servers.filter((server) => key.split(",").includes(server.id) && server.fingerprint);
    void Promise.all(
      pinned.map(async (server) => [server.id, await probePin(server.baseUrl, server.fingerprint ?? "")] as const),
    ).then((results) => {
      if (!cancelled) setChanged(new Set(results.filter(([, result]) => result === "mismatch").map(([id]) => id)));
    });
    return () => {
      cancelled = true;
    };
  }, [key, servers]);
  return changed;
}

export function OfflineHosts() {
  const tokens = useTokens();
  const { selectServer } = useFleetScope();
  const { setServerDisabled } = useAuth();
  const statuses = useServerFleetStatuses();
  const { signedIn } = useAnywhere();
  const { hosts } = useAnywhereHosts();
  const unreachable = statuses.filter((status) => status.reachability === "unreachable");
  const unreachableIds = useMemo(() => unreachable.map((server) => server.id), [unreachable]);
  const certChanged = useChangedCertificates(unreachableIds);
  const disabled = statuses.filter((status) => status.reachability === "disabled");
  const away = signedIn ? hosts.filter((host) => host.state.kind === "offline" || host.state.kind === "stale") : [];
  if (unreachable.length === 0 && away.length === 0 && disabled.length === 0) return null;

  return (
    <View style={styles.section}>
      <Text style={[typeScale.section, styles.label, { color: tokens.ink4 }]}>OFFLINE HOSTS</Text>
      {unreachable.map((server) => (
        <Pressable
          key={server.id}
          onPress={() => (certChanged.has(server.id) ? router.push("/connect") : selectServer(server.id))}
          accessibilityRole="button"
          accessibilityLabel={`${server.name}, ${certChanged.has(server.id) ? "certificate changed, re-pair" : `unreachable, ${lastSeenLabel(server.lastSeenMs)}`}`}
          accessibilityHint={certChanged.has(server.id) ? "Opens pairing to scan a new code" : "Shows this server and retries it"}
          style={[styles.row, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
        >
          <View style={[styles.dot, { backgroundColor: tokens.danger }]} />
          <Text style={[typeScale.body, styles.name, { color: tokens.ink2 }]} numberOfLines={1}>
            {server.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
            {certChanged.has(server.id) ? "certificate changed, re-pair" : `offline · ${lastSeenLabel(server.lastSeenMs)}`}
          </Text>
        </Pressable>
      ))}
      {disabled.map((server) => (
        <View
          key={server.id}
          style={[styles.row, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
          accessibilityLabel={`${server.name}, disabled, ${lastSeenLabel(server.lastSeenMs)}`}
        >
          <View style={[styles.dot, { backgroundColor: tokens.ink4 }]} />
          <Text style={[typeScale.body, styles.name, { color: tokens.ink3 }]} numberOfLines={1}>
            {server.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            disabled · {lastSeenLabel(server.lastSeenMs)}
          </Text>
          <Button
            label="Re-enable"
            variant="secondary"
            onPress={() => void setServerDisabled(server.id, false)}
            accessibilityLabel={`Re-enable ${server.name}`}
          />
        </View>
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
