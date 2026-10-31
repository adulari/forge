import { Wifi } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { findNearbyHosts, type NearbyHost } from "../../lib/pinnedLan";
import { isTauri } from "../../lib/platform";
import { useTokens } from "../../theme/ThemeProvider";
import { space, tapTarget } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export function NearbyHosts({ onPick }: { onPick: (host: NearbyHost) => void }) {
  const tokens = useTokens();
  const [hosts, setHosts] = useState<NearbyHost[]>([]);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    const scan = () =>
      void findNearbyHosts().then((found) => {
        if (!cancelled) setHosts(found);
      });
    scan();
    const timer = setInterval(scan, 8000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, []);

  if (hosts.length === 0) return null;
  const nameCounts = new Map<string, number>();
  for (const host of hosts) nameCounts.set(host.name, (nameCounts.get(host.name) ?? 0) + 1);
  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.section, { color: tokens.ink4 }]}>nearby (unverified)</Text>
      {hosts.map((host) => (
        <Pressable
          key={`${host.host}:${host.port}`}
          onPress={() => onPick(host)}
          accessibilityRole="button"
          accessibilityLabel={`Connect to ${host.name}`}
          style={styles.row}
        >
          <Wifi size={16} strokeWidth={1.75} color={tokens.accent} />
          <Text style={[typeScale.sub, styles.name, { color: tokens.ink }]} numberOfLines={1}>
            {(nameCounts.get(host.name) ?? 0) > 1 ? `${host.name} (same name as another host)` : host.name}
          </Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{`${host.host}:${host.port}`}</Text>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { width: "100%", maxWidth: 480, gap: space.space4 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget },
  name: { flex: 1 },
});
