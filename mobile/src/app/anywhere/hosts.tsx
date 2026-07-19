// Forge Anywhere — Hosts list (mobile.dc.html "AW Hosts" lines 343-404). The design's
// "other host states" block (lines 389-398) is a reference showcase for the design doc
// itself, not a real screen section — skipped per BUILD instructions; the mock host seed
// (mockClient.ts) already covers online/busy/stale/revoked so the real row below is
// exercised against every HostState that matters.
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { ChevronRight, Copy } from "lucide-react-native";
import React, { useCallback, useEffect } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { SettingsShell } from "../(tabs)/settings";
import { HostDot } from "../../components/anywhere/HostDot";
import { BackLink } from "../../components/ds/BackLink";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { SkeletonRow } from "../../components/ds/Skeleton";
import { useToast } from "../../components/ds/ToastHost";
import { goBackOr } from "../../lib/nav";
import { hostStateText } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { MAX_ACTIVE_HOSTS, type AnywhereHost, type HostState } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { useStrike } from "../../theme/motion";
import { radii, rowHeight, space, type ColorTokens } from "../../theme/tokens";
import { type as typeScale, tabularNums } from "../../theme/typography";

const ADD_HOST_COMMAND = "forge anywhere enable --name NAME";

function hostMetaColor(state: HostState, tokens: ColorTokens): string {
  switch (state.kind) {
    case "stale":
    case "update-required":
      return tokens.warn;
    case "revoked":
    case "disabled":
    case "offline":
      return tokens.ink4;
    default:
      return tokens.ink3;
  }
}

function HostRow({ host, showSeparator }: { host: AnywhereHost; showSeparator: boolean }) {
  const tokens = useTokens();
  const { style: strikeStyle, onPressIn, onPressOut } = useStrike();
  const revoked = host.state.kind === "revoked";

  const row = (
    <Animated.View style={[styles.row, revoked ? undefined : strikeStyle]}>
      <HostDot state={host.state} />
      <Text
        style={[
          typeScale.bodyBold,
          styles.name,
          { color: revoked ? tokens.ink3 : tokens.ink },
          revoked && styles.strikethrough,
        ]}
        numberOfLines={1}
      >
        {host.name}
      </Text>
      <Text style={[typeScale.monoMeta, tabularNums, { color: hostMetaColor(host.state, tokens) }]} numberOfLines={1}>
        {hostStateText(host)}
      </Text>
      {!revoked ? <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink4} /> : null}
    </Animated.View>
  );

  return (
    <View>
      {revoked ? (
        row
      ) : (
        <Pressable
          onPress={() => router.push({ pathname: "/anywhere/host/[id]", params: { id: host.id } })}
          onPressIn={onPressIn}
          onPressOut={onPressOut}
          accessibilityRole="button"
          accessibilityLabel={`${host.name} — ${hostStateText(host)}`}
        >
          {row}
        </Pressable>
      )}
      {showSeparator ? <View style={[styles.separator, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

function HostsSkeleton() {
  return (
    <View style={styles.skeletonWrap}>
      <SkeletonRow />
      <SkeletonRow />
      <SkeletonRow />
    </View>
  );
}

export default function AnywhereHostsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { signedIn, loading: accountLoading } = useAnywhere();
  const { hosts, loading } = useAnywhereHosts();

  useEffect(() => {
    if (!accountLoading && !signedIn) router.replace("/anywhere");
  }, [accountLoading, signedIn]);

  const activeCount = hosts.filter((h) => h.state.kind !== "revoked" && h.state.kind !== "disabled").length;

  const onCopyCommand = useCallback(async () => {
    await Clipboard.setStringAsync(ADD_HOST_COMMAND);
    toast.show("Command copied.", { tone: "neutral" });
  }, [toast]);

  if (!signedIn) return null;

  return (
    <SettingsShell active="anywhere">
      <Screen scroll contentContainerStyle={styles.content}>
        <View style={styles.headerRow}>
          <BackLink label="Anywhere" onPress={() => goBackOr("/anywhere")} />
          <Text style={[typeScale.headingBold, styles.headerTitle, { color: tokens.ink }]}>Hosts</Text>
          {!loading ? (
            <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{`${activeCount} of ${MAX_ACTIVE_HOSTS} active`}</Text>
          ) : null}
        </View>

        {loading ? (
          <HostsSkeleton />
        ) : (
          <View style={styles.section}>
            {hosts.map((host, index) => (
              <HostRow key={host.id} host={host} showSeparator={index < hosts.length - 1} />
            ))}
          </View>
        )}

        <View style={styles.section}>
          <SectionHeader>Add a host</SectionHeader>
          <Pressable
            onPress={onCopyCommand}
            accessibilityRole="button"
            accessibilityLabel="Copy command to enable Anywhere on a host"
            style={[styles.commandBox, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
          >
            <Text style={[typeScale.codeSmall, tabularNums, styles.commandText, { color: tokens.ink }]} numberOfLines={1}>
              <Text style={{ color: tokens.ink3 }}>$ </Text>
              {ADD_HOST_COMMAND}
            </Text>
            <Copy size={14} strokeWidth={1.75} color={tokens.ink2} />
          </Pressable>
          <Text style={[typeScale.meta, styles.limitNote, { color: tokens.ink3 }]}>
            Anywhere includes up to 3 active hosts. At the limit, disable or revoke one first — its local Forge keeps
            working either way.
          </Text>
        </View>
      </Screen>
    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space4 },
  headerTitle: { flex: 1 },
  section: { marginTop: space.space20 },
  row: {
    minHeight: rowHeight.list,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space4,
    gap: 10,
  },
  name: { flex: 1 },
  strikethrough: { textDecorationLine: "line-through" },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: space.space16 },
  skeletonWrap: { marginTop: space.space16, gap: space.space8 },
  commandBox: {
    marginTop: space.space8,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius12,
    paddingHorizontal: space.space12,
    paddingVertical: space.space12,
  },
  commandText: { flex: 1 },
  limitNote: { marginTop: space.space8, lineHeight: 16 },
});
