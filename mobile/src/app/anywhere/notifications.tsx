// Forge Anywhere — push notification preferences (mobile.dc.html "AW Notifications",
// lines 1031-1077). The toggle/permission rows reuse the app's real push infrastructure
// (mirrors (tabs)/settings.tsx's onPushChange) since that state is genuinely live;
// the relay-specific rows the design also shows ("refreshing token", "relay
// unavailable") have no backing signal in AnywhereClient/mockClient today, so they
// stay as static reference rows — never auto-selected as the current state.
import { router } from "expo-router";
import React, { useCallback, useEffect, useState } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";
import { Flame } from "lucide-react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Screen } from "../../components/ds/Screen";
import { Switch } from "../../components/ds/Switch";
import { useToast } from "../../components/ds/ToastHost";
import { useAuth } from "../../lib/auth";
import { checkNotifyPermission, type NotifyPermission } from "../../lib/notify";
import { enablePush, disablePush, getPushStatus, initPush, isPushSupported, type PushSubscriptionState } from "../../lib/push";
import { useAnywhere } from "../../lib/anywhere/store";
import { isIOS, isTauri, isWeb } from "../../lib/platform";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

const NOTIFICATIONS_SUPPORTED = (isWeb && !isTauri) || isIOS;

type StateKey = "enabled-healthy" | "permission-denied" | "refreshing" | "relay-unavailable" | "disabled";

function StateRow({ dotColor, pulsing, text, actionLabel, onAction, current, showSeparator }: {
  dotColor: string;
  pulsing?: boolean;
  text: string;
  actionLabel?: string;
  onAction?: () => void;
  current: boolean;
  showSeparator: boolean;
}) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(pulsing ? "busy" : "idle");
  return (
    <View>
      <View style={styles.stateRow}>
        <Animated.View style={[styles.dot, pulsing ? dotStyle : undefined, { backgroundColor: dotColor }]} />
        <Text style={[type.sub, styles.stateText, { color: current ? tokens.ink : tokens.ink3 }]}>{text}</Text>
        {actionLabel && onAction ? (
          <Pressable onPress={onAction} accessibilityRole="button" accessibilityLabel={actionLabel} hitSlop={6}>
            <Text style={[type.meta, { color: tokens.accent }]}>{actionLabel}</Text>
          </Pressable>
        ) : null}
      </View>
      {showSeparator ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
    </View>
  );
}

export default function AnywhereNotificationsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { signedIn, loading } = useAnywhere();
  const { baseUrl } = useAuth();

  const [pushStatus, setPushStatus] = useState<PushSubscriptionState>("unsupported");
  const [pushLoaded, setPushLoaded] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const [notifyPermission, setNotifyPermission] = useState<NotifyPermission>("default");
  const pushSupported = NOTIFICATIONS_SUPPORTED && isPushSupported();

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  useEffect(() => {
    if (!NOTIFICATIONS_SUPPORTED) return;
    let cancelled = false;
    void initPush();
    getPushStatus().then((status) => {
      if (cancelled) return;
      setPushStatus(status);
      setPushLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    checkNotifyPermission().then((permission) => {
      if (!cancelled) setNotifyPermission(permission);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const onToggle = useCallback(
    async (value: boolean) => {
      if (!baseUrl || pushBusy) return;
      setPushBusy(true);
      try {
        const next = value ? await enablePush(baseUrl) : await disablePush(baseUrl);
        setPushStatus(next);
        if (value && next !== "subscribed") {
          toast.show("Couldn't enable notifications — check the system permission prompt.", { tone: "danger" });
        }
      } finally {
        setPushBusy(false);
      }
    },
    [baseUrl, pushBusy, toast],
  );

  const currentState: StateKey = !pushSupported
    ? "disabled"
    : pushBusy
      ? "refreshing"
      : isTauri && notifyPermission === "denied"
        ? "permission-denied"
        : pushStatus === "subscribed"
          ? "enabled-healthy"
          : "disabled";

  if (loading || !signedIn) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
        <Text style={[type.headingBold, { color: tokens.ink }]}>Notifications</Text>
      </View>

      <View style={styles.toggleSection}>
        <View style={styles.toggleRow}>
          <Text style={[type.body, styles.toggleLabel, { color: tokens.ink }]}>Push notifications</Text>
          {pushSupported && pushLoaded ? (
            <Switch
              value={pushStatus === "subscribed"}
              onValueChange={(v) => void onToggle(v)}
              disabled={pushBusy || !baseUrl}
              accessibilityLabel="Push notifications"
            />
          ) : null}
        </View>
        <Text style={[type.meta, styles.privacyCopy, { color: tokens.ink3 }]}>
          Pushes are generic on purpose — never prompts, filenames, repos, diffs or transcript
          content. The app decrypts and routes you after opening.
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>lock screen — exact copy</Text>
        <View style={[styles.previewCard, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
          <View style={styles.previewRow}>
            <View style={[styles.previewIcon, { backgroundColor: tokens.bg1 }]}>
              <Flame size={14} color={tokens.accent} fill={tokens.accent} />
            </View>
            <View style={styles.previewBody}>
              <Text style={[type.bodyBold, { color: tokens.ink }]}>Forge</Text>
              <Text style={[type.sub, { color: tokens.ink2 }]}>Open Forge to view an update.</Text>
            </View>
            <Text style={[type.monoMeta, { color: tokens.ink4 }]}>now</Text>
          </View>
        </View>
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>states</Text>
        <StateRow
          dotColor={tokens.success}
          text="Enabled · token healthy"
          current={currentState === "enabled-healthy"}
          showSeparator
        />
        <StateRow
          dotColor={tokens.danger}
          text="Permission denied"
          actionLabel={!isWeb ? "Open Settings" : undefined}
          onAction={!isWeb ? () => void Linking.openSettings() : undefined}
          current={currentState === "permission-denied"}
          showSeparator
        />
        <StateRow dotColor={tokens.accent} pulsing text="Refreshing push token…" current={currentState === "refreshing"} showSeparator />
        <StateRow dotColor={tokens.warn} text="Relay unavailable — alerts pause, sessions unaffected" current={false} showSeparator />
        <StateRow dotColor={tokens.ink4} text="Disabled — you'll only see updates in-app" current={currentState === "disabled"} showSeparator={false} />
      </View>

      <View style={styles.section}>
        <Text style={[type.section, { color: tokens.ink4 }]}>on open</Text>
        <Text style={[type.meta, styles.onOpenCopy, { color: tokens.ink3 }]}>
          The app refreshes, decrypts, and lands you on the relevant surface — a waiting
          permission opens its decision card, a finished job opens its session.
        </Text>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  header: { gap: space.space4 },
  toggleSection: { marginTop: space.space12 },
  toggleRow: { flexDirection: "row", alignItems: "center", paddingVertical: space.space8 },
  toggleLabel: { flex: 1 },
  privacyCopy: { lineHeight: 18 },
  section: { marginTop: space.space20 },
  previewCard: { marginTop: space.space12, borderWidth: 1, borderRadius: radii.radius16, padding: space.space12 },
  previewRow: { flexDirection: "row", alignItems: "center", gap: 9 },
  previewIcon: { width: 26, height: 26, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center" },
  previewBody: { flex: 1, gap: 1 },
  stateRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: 11 },
  stateText: { flex: 1 },
  dot: { width: 7, height: 7, borderRadius: 3.5 },
  hairline: { height: StyleSheet.hairlineWidth, marginLeft: 17 },
  onOpenCopy: { marginTop: space.space8, lineHeight: 18 },
});
