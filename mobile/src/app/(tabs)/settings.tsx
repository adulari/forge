// Settings (FEATURES.md §4 IA, BUILD_ORDER.md T2.2). Four sections: Servers
// (multi-daemon list from `useAuth()`), Appearance (theme preference), Security
// (app-lock toggle), About/Diagnostics (version, protocol, active server).
//
// Server removal is via the trailing delete IconButton + ConfirmDialog
// (ds/ListRow has no swipe-action variant yet — that's a fleet/SessionCard-
// level concern per BUILD_ORDER T2.3, out of this task's scope).
//
// HANDOFF(T2.2): the app-lock preference key is `forge.appLock` (AsyncStorage,
// boolean "true"/"false") — the redesigned `src/components/AppLock.tsx` (owned
// by T2.1, "port the existing Face ID gate onto ds primitives") must read/write
// this same key so the Settings toggle and the lock gate agree. The current
// legacy AppLock.tsx (pre-redesign) uses a different key (`forge.biometricLockEnabled`)
// and is out of this task's file scope.
import AsyncStorage from "@react-native-async-storage/async-storage";
import Constants from "expo-constants";
import { router } from "expo-router";
import { Plus, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Badge } from "../../components/ds/Badge";
import { Card } from "../../components/ds/Card";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { IconButton } from "../../components/ds/IconButton";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Segmented } from "../../components/ds/Segmented";
import { Switch } from "../../components/ds/Switch";
import { useToast } from "../../components/ds/ToastHost";
import { type StoredServer, useAuth } from "../../lib/auth";
import {
  enablePush,
  disablePush,
  getPushStatus,
  initPush,
  isPushSupported,
  type PushSubscriptionState,
} from "../../lib/push";
import { isTauri, isWeb } from "../../lib/platform";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

const NOTIFICATIONS_SUPPORTED = isWeb && !isTauri;

const APP_LOCK_KEY = "forge.appLock";

function maskToken(token: string | null): string {
  if (!token || token.length < 4) return "—";
  return `…${token.slice(-4)}`;
}

export default function SettingsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { preference, setScheme } = useTheme();
  const { baseUrl, servers, activeServerId, host, token: activeToken, setActive, removeServer } = useAuth();

  const [appLock, setAppLock] = useState(false);
  const [appLockLoaded, setAppLockLoaded] = useState(false);
  const [pendingRemove, setPendingRemove] = useState<StoredServer | null>(null);

  const [pushStatus, setPushStatus] = useState<PushSubscriptionState>("unsupported");
  const [pushLoaded, setPushLoaded] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const pushSupported = NOTIFICATIONS_SUPPORTED && isPushSupported();

  useEffect(() => {
    let cancelled = false;
    AsyncStorage.getItem(APP_LOCK_KEY).then((raw) => {
      if (cancelled) return;
      setAppLock(raw === "true");
      setAppLockLoaded(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

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

  const onAppLockChange = (value: boolean) => {
    setAppLock(value);
    void AsyncStorage.setItem(APP_LOCK_KEY, value ? "true" : "false");
  };

  const onPushChange = useCallback(
    async (value: boolean) => {
      if (!baseUrl || pushBusy) return;
      setPushBusy(true);
      try {
        const next = value ? await enablePush(baseUrl) : await disablePush(baseUrl);
        setPushStatus(next);
        if (value && next !== "subscribed") {
          toast.show("couldn't enable notifications — check the browser's permission prompt.", {
            tone: "danger",
          });
        }
      } finally {
        setPushBusy(false);
      }
    },
    [baseUrl, pushBusy, toast],
  );

  const appVersion = Constants.expoConfig?.version ?? "—";

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <Text style={[type.title, styles.pageTitle, { color: tokens.ink }]}>Settings</Text>

      <View>
        <SectionHeader>Servers</SectionHeader>
        <Card padded={false}>
          {servers.map((server) => (
            <ListRow
              key={server.id}
              title={server.name}
              subtitle={maskToken(server.token)}
              onPress={() => setActive(server.id)}
              leading={
                server.id === activeServerId ? <Badge label="active" tone="accent" /> : undefined
              }
              trailing={
                <IconButton
                  icon={<Trash2 size={20} strokeWidth={1.75} color={tokens.ink3} />}
                  accessibilityLabel={`Remove server ${server.name}`}
                  onPress={() => setPendingRemove(server)}
                />
              }
              hasInteractiveTrailing
            />
          ))}
          <ListRow
            title="Add server"
            leading={<Plus size={20} strokeWidth={1.75} color={tokens.accent} />}
            onPress={() => router.push("/connect")}
            showSeparator={false}
          />
        </Card>
      </View>

      <View>
        <SectionHeader>Appearance</SectionHeader>
        <Card style={styles.appearanceCard}>
          <Segmented
            options={[
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
              { value: "system", label: "System" },
            ]}
            value={preference}
            onChange={setScheme}
          />
        </Card>
      </View>

      <View>
        <SectionHeader>Security</SectionHeader>
        <Card padded={false}>
          <ListRow
            title="Require Face ID"
            subtitle="Lock Forge behind biometric authentication when you return to it."
            showSeparator={false}
            trailing={
              appLockLoaded ? (
                <Switch value={appLock} onValueChange={onAppLockChange} accessibilityLabel="Require Face ID" />
              ) : undefined
            }
          />
        </Card>
      </View>

      {NOTIFICATIONS_SUPPORTED ? (
        <View>
          <SectionHeader>Notifications</SectionHeader>
          <Card padded={false}>
            <ListRow
              title="Web push"
              subtitle={
                !pushSupported
                  ? "not supported in this browser."
                  : pushStatus === "subscribed"
                    ? "Allow/Deny prompts reach you here, even with this tab closed."
                    : "get notified in this browser when a session needs you."
              }
              showSeparator={false}
              trailing={
                pushSupported && pushLoaded ? (
                  <Switch
                    value={pushStatus === "subscribed"}
                    onValueChange={onPushChange}
                    disabled={pushBusy || !baseUrl}
                    accessibilityLabel="Web push notifications"
                  />
                ) : undefined
              }
            />
          </Card>
        </View>
      ) : null}

      <View>
        <SectionHeader>About &amp; diagnostics</SectionHeader>
        <Card padded={false}>
          <KeyValueRow label="Version" value={appVersion} />
          <KeyValueRow label="Protocol" value="v7" />
          <KeyValueRow label="Active server" value={host ? `${host} · ${maskToken(activeToken)}` : "none"} />
        </Card>
      </View>

      <ConfirmDialog
        visible={pendingRemove != null}
        title={`Remove ${pendingRemove?.name ?? "server"}?`}
        message="You'll need to re-pair to use it again — its stored token is deleted from this device."
        confirmLabel="Remove"
        destructive
        onConfirm={() => {
          if (pendingRemove) removeServer(pendingRemove.id);
          setPendingRemove(null);
        }}
        onCancel={() => setPendingRemove(null)}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space32, gap: space.space20 },
  pageTitle: { paddingHorizontal: space.space4 },
  appearanceCard: { gap: space.space8 },
});
