// Forge Anywhere — connect first host (mobile.dc.html "AW First Host", lines 253-292).
import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { Copy } from "lucide-react-native";
import React, { useEffect } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { Screen } from "../../components/ds/Screen";
import { useToast } from "../../components/ds/ToastHost";
import { goBackOr } from "../../lib/nav";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";

const ENABLE_COMMAND = "forge anywhere enable --name atlas";

export default function AnywhereFirstHostScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { signedIn, loading } = useAnywhere();
  const { hosts, loading: hostsLoading } = useAnywhereHosts();
  const { dotStyle } = useEmberdot("busy");

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  // The mock backend seeds hosts unconditionally rather than only after a real
  // `forge anywhere enable` run — so on this mock, the "waiting" state resolves as
  // soon as the first listHosts() fetch lands, rather than genuinely waiting for a
  // connector heartbeat. A real relay backend starts this list empty.
  useEffect(() => {
    if (loading || !signedIn || hostsLoading) return;
    if (hosts.length > 0) {
      toast.show(`${hosts[0].name} connected.`, { tone: "success" });
      router.replace("/anywhere");
    }
    // toast is stable from useToast(); re-firing on hosts/loading changes is the point.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hosts, hostsLoading, loading, signedIn]);

  const onCopy = async () => {
    await Clipboard.setStringAsync(ENABLE_COMMAND);
    toast.show("Command copied.");
  };

  if (loading || !signedIn) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <Pressable
          onPress={() => goBackOr("/anywhere")}
          accessibilityRole="button"
          accessibilityLabel="Back"
          hitSlop={8}
          style={styles.backHit}
        >
          <Text style={[type.bodyBold, { color: tokens.ink2 }]}>‹</Text>
        </Pressable>
        <Text style={[type.headingBold, { color: tokens.ink }]}>Connect your first host</Text>
      </View>
      <Text style={[type.sub, styles.intro, { color: tokens.ink2 }]}>
        On the machine that runs Forge, enable Anywhere:
      </Text>

      <View style={[styles.commandBox, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
        <Text style={[type.code, styles.commandText, { color: tokens.ink }]} numberOfLines={1}>
          <Text style={{ color: tokens.ink3 }}>$ </Text>
          {ENABLE_COMMAND}
        </Text>
        <Pressable onPress={onCopy} accessibilityRole="button" accessibilityLabel="Copy command" hitSlop={8}>
          <Copy size={14} strokeWidth={2} color={tokens.ink2} />
        </Pressable>
      </View>
      <Text style={[type.meta, styles.explainer, { color: tokens.ink3 }]}>
        The host signs in with the same GitHub account, gets a stable identity, and connects
        to the relay. Pick any name — rename anytime from Host details; identity never changes.
      </Text>

      <View style={styles.waitingRow}>
        <Animated.View style={[styles.pulseDot, dotStyle, { backgroundColor: tokens.accent }]} />
        <Text style={[type.sub, { color: tokens.ink2 }]}>Waiting for a host to connect…</Text>
      </View>
      <Text style={[type.sub, styles.trialNote, { color: tokens.ink3 }]}>
        Your 14-day trial has not started yet. It begins when the first host connects — no card
        required.
      </Text>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>while you wait</Text>
        <WhileYouWaitRow
          label="Use Forge over Direct — nothing here blocks local work"
          actionLabel="Fleet"
          onPress={() => router.push("/(tabs)")}
        />
        <WhileYouWaitRow
          label="Pair another controller device"
          actionLabel="Pair"
          onPress={() => router.push("/anywhere/pair")}
          showSeparator={false}
        />
      </View>
    </Screen>
  );
}

function WhileYouWaitRow({
  label,
  actionLabel,
  onPress,
  showSeparator = true,
}: {
  label: string;
  actionLabel: string;
  onPress: () => void;
  showSeparator?: boolean;
}) {
  const tokens = useTokens();
  return (
    <View>
      <View style={styles.waitRow}>
        <Text style={[type.sub, styles.waitLabel, { color: tokens.ink2 }]}>{label}</Text>
        <Pressable onPress={onPress} accessibilityRole="button" accessibilityLabel={actionLabel} hitSlop={8}>
          <Text style={[type.meta, { color: tokens.accent }]}>{actionLabel}</Text>
        </Pressable>
      </View>
      {showSeparator ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: tapTarget },
  backHit: { width: tapTarget, height: tapTarget, alignItems: "center", justifyContent: "center", marginLeft: -space.space8 },
  intro: { marginLeft: tapTarget, marginTop: -space.space8 },
  commandBox: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
    marginTop: space.space16,
    borderWidth: 1,
    borderRadius: radii.radius12,
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
  },
  commandText: { flex: 1 },
  explainer: { marginTop: space.space8, lineHeight: 17 },
  waitingRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space24 },
  pulseDot: { width: 8, height: 8, borderRadius: 4 },
  trialNote: { marginTop: space.space12, lineHeight: 19 },
  section: { marginTop: space.space24 },
  waitRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12 },
  waitLabel: { flex: 1 },
  hairline: { height: StyleSheet.hairlineWidth },
});
