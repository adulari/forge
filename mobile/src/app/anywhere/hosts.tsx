// Forge Anywhere — Hosts list (mobile.dc.html "AW Hosts Detail Pair" lines 496-505).
// Real host list from AnywhereProvider. The comp shows online/busy/stale/revoked —
// the real AnywhereHost only reports online vs. last-heartbeat, so this renders the
// two states it actually has rather than inventing busy/stale/revoked distinctions
// no backend field backs yet.
import { router } from "expo-router";
import { ChevronRight } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import { hostStatusText } from "../../lib/anywhereHostPresence";
import { MAX_ACTIVE_HOSTS } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

export default function AnywhereHostsScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();

  if (anywhere.phase !== "ready") return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.shell}>
        <View style={styles.header}>
          <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
          <View style={styles.headerRow}>
            <Text accessibilityRole="header" style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>
              Hosts
            </Text>
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
              {`${anywhere.hosts.length} of ${MAX_ACTIVE_HOSTS} active`}
            </Text>
          </View>
        </View>

        <View style={styles.list}>
          {anywhere.hosts.map((host) => (
            <Pressable
              key={host.id}
              onPress={() => router.push({ pathname: "/anywhere/host/[id]", params: { id: host.id } })}
              accessibilityRole="button"
              accessibilityLabel={host.name}
              style={[styles.row, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
            >
              <View style={[styles.dot, { backgroundColor: host.online === true ? tokens.success : tokens.ink4 }]} />
              <Text style={[typeScale.body, styles.rowLabel, { color: tokens.ink }]} numberOfLines={1}>
                {host.name}
              </Text>
              <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
                {hostStatusText(host)}
              </Text>
              <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink4} />
            </Pressable>
          ))}
          {!anywhere.hosts.length ? (
            <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>
              No hosts yet. Enable Anywhere on a machine running Forge to add one.
            </Text>
          ) : null}
        </View>

        <Text style={[typeScale.monoMeta, styles.footnote, { color: tokens.ink4 }]}>
          {`add: $ forge anywhere enable --name NAME · ${MAX_ACTIVE_HOSTS} active max — disable or revoke to free a slot`}
        </Text>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  shell: { width: "100%", maxWidth: 680, alignSelf: "center" },
  header: { gap: space.space8, marginBottom: space.space4 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headerTitle: { flex: 1 },
  list: { marginTop: space.space12, gap: space.space8 },
  row: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    paddingHorizontal: space.space16,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 3,
  },
  dot: { width: 6, height: 6, borderRadius: 3 },
  rowLabel: { flex: 1, fontWeight: "600" },
  empty: { paddingVertical: space.space16 },
  footnote: { marginTop: space.space16, lineHeight: 16, fontFamily: monoFamily.regular },
});
