// Settings (FEATURES.md §4 IA, BUILD_ORDER.md T2.2). Four sections: Servers
// (multi-daemon list from `useAuth()`), Appearance (theme preference), Security
// (app-lock toggle), About/Diagnostics (version, protocol, active server).
//
// Server removal is via the trailing delete IconButton + ConfirmDialog
// (ds/ListRow has no swipe-action variant yet — that's a fleet/SessionCard-
// level concern per BUILD_ORDER T2.3, out of this task's scope).
//
// The app-lock preference key is `forge.appLock` (AsyncStorage, boolean "true"/"false") —
// `src/components/AppLock.tsx` reads/writes the same key so the Settings toggle and the
// lock gate agree.
import AsyncStorage from "@react-native-async-storage/async-storage";
import Constants from "expo-constants";
import { router } from "expo-router";
import { Bell, Plus, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Badge, type BadgeTone } from "../../components/ds/Badge";
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
import { checkNotifyPermission, getNotifyPermission, notify, type NotifyPermission } from "../../lib/notify";
import {
  enablePush,
  disablePush,
  getPushStatus,
  initPush,
  isPushSupported,
  type PushSubscriptionState,
} from "../../lib/push";
import { ApiError } from "../../lib/api";
import { haptics, initHaptics, isHapticsEnabled, setHapticsEnabled } from "../../lib/haptics";
import { useServerFleets } from "../../lib/queries";
import { isIOS, isTauri, isWeb } from "../../lib/platform";
import { checkForDesktopUpdate, type DesktopUpdate } from "../../lib/updater";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

const NOTIFICATIONS_SUPPORTED = (isWeb && !isTauri) || isIOS;

// Tauri gets its own row instead of folding into NOTIFICATIONS_SUPPORTED above: desktop
// notifications go through notify.ts's OS-native path (tauri-plugin-notification), not
// the web-push subscription flow the rest of this section drives.
function notifyPermissionTone(permission: NotifyPermission): BadgeTone {
  switch (permission) {
    case "granted":
      return "success";
    case "denied":
      return "danger";
    case "unsupported":
      return "neutral";
    case "default":
    default:
      return "warn";
  }
}

function notifyPermissionLabel(permission: NotifyPermission): string {
  switch (permission) {
    case "granted":
      return "allowed";
    case "denied":
      return "blocked";
    case "unsupported":
      return "unsupported";
    case "default":
    default:
      return "not requested";
  }
}

const APP_LOCK_KEY = "forge.appLock";

function maskToken(token: string | null): string {
  if (!token || token.length < 4) return "—";
  return `…${token.slice(-4)}`;
}

export default function SettingsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { preference, setScheme } = useTheme();
  const { baseUrl, servers, activeServerId, host, token: activeToken, setActive, removeServer, testConnection } = useAuth();

  const serverQueries = useServerFleets(servers);
  const [appLock, setAppLock] = useState(false);
  const [appLockLoaded, setAppLockLoaded] = useState(false);
  const [hapticsEnabled, setHapticsEnabledState] = useState(isHapticsEnabled);
  const [pendingRemove, setPendingRemove] = useState<StoredServer | null>(null);
  const [health, setHealth] = useState<"idle" | "checking" | "ok" | "bad-token" | "unreachable" | "server-error">("idle");

  const [pushStatus, setPushStatus] = useState<PushSubscriptionState>("unsupported");
  const [pushLoaded, setPushLoaded] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const pushSupported = NOTIFICATIONS_SUPPORTED && isPushSupported();

  const [notifyPermission, setNotifyPermission] = useState<NotifyPermission>("default");
  const [notifyBusy, setNotifyBusy] = useState(false);
  const [desktopUpdate, setDesktopUpdate] = useState<DesktopUpdate | null>(null);
  const [updateBusy, setUpdateBusy] = useState(false);

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
    let cancelled = false;
    void initHaptics().then((value) => {
      if (!cancelled) setHapticsEnabledState(value);
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

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    checkNotifyPermission().then((permission) => {
      if (cancelled) return;
      setNotifyPermission(permission);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    if (!baseUrl) {
      setHealth("idle");
      return;
    }
    setHealth("checking");
    testConnection().then((result) => {
      // testConnection's in-flight state is "testing"; this screen models that as "checking".
      if (!cancelled) setHealth(result === "testing" ? "checking" : result);
    });
    return () => {
      cancelled = true;
    };
  }, [activeServerId, baseUrl, testConnection]);

  const healthCopy = health === "ok" ? "online" : health === "checking" ? "checking connection…" : health === "bad-token" ? "pairing invalid — re-scan or remove this server" : health === "unreachable" ? "offline — check that forge serve is running" : health === "server-error" ? "server error — check forge serve logs" : "not connected";
  const healthTone: BadgeTone = health === "ok" ? "success" : health === "idle" ? "neutral" : health === "checking" ? "warn" : "danger";

  const onAppLockChange = (value: boolean) => {
    setAppLock(value);
    void AsyncStorage.setItem(APP_LOCK_KEY, value ? "true" : "false");
  };

  const onHapticsChange = (value: boolean) => {
    setHapticsEnabledState(value);
    setHapticsEnabled(value);
    if (value) haptics.sendPrompt();
  };

  const onPushChange = useCallback(
    async (value: boolean) => {
      if (!baseUrl || pushBusy) return;
      setPushBusy(true);
      try {
        const next = value ? await enablePush(baseUrl) : await disablePush(baseUrl);
        setPushStatus(next);
        if (value && next !== "subscribed") {
          toast.show(
            isIOS
              ? "couldn't enable notifications — check Settings > Forge > Notifications is allowed."
              : "couldn't enable notifications — check the browser's permission prompt.",
            { tone: "danger" },
          );
        }
      } catch (err) {
        // A thrown error (vs. a resolved non-"subscribed" state) means permission was fine but
        // the subscribe/unsubscribe call itself failed — say so, rather than reusing the
        // permission-denied copy above.
        toast.show(err instanceof ApiError ? err.message : "couldn't reach the server — try again.", {
          tone: "danger",
        });
      } finally {
        setPushBusy(false);
      }
    },
    [baseUrl, pushBusy, toast],
  );

  const onSendTestNotification = useCallback(async () => {
    if (notifyBusy) return;
    setNotifyBusy(true);
    try {
      await notify("Forge", "Test notification — this is what a session alert looks like.");
      const permission = getNotifyPermission();
      setNotifyPermission(permission);
      if (permission !== "granted") {
        toast.show("couldn't send — notifications are blocked for Forge in System Settings.", {
          tone: "danger",
        });
      }
    } finally {
      setNotifyBusy(false);
    }
  }, [notifyBusy, toast]);

  useEffect(() => {
    if (!isTauri) return;
    void checkForDesktopUpdate().then(setDesktopUpdate).catch(() => undefined);
  }, []);

  const checkDesktopUpdate = useCallback(async () => {
    setUpdateBusy(true);
    try {
      const update = await checkForDesktopUpdate();
      setDesktopUpdate(update);
      if (!update) toast.show("Forge is up to date.", { tone: "neutral" });
    } catch {
      toast.show("couldn't check for updates.", { tone: "danger" });
    } finally {
      setUpdateBusy(false);
    }
  }, [toast]);

  const installDesktopUpdate = useCallback(async () => {
    if (!desktopUpdate) return;
    setUpdateBusy(true);
    try { await desktopUpdate.install(); } catch { toast.show("couldn't install update.", { tone: "danger" }); }
    finally { setUpdateBusy(false); }
  }, [desktopUpdate, toast]);
  const appVersion = Constants.expoConfig?.version ?? "—";

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <Text style={[type.title, styles.pageTitle, { color: tokens.ink }]}>Settings</Text>

      <View>
        <SectionHeader>Servers</SectionHeader>
        <Card padded={false}>
          {servers.map((server, index) => {
            const fleet = serverQueries[index];
            const rows = fleet.data ?? [];
            const reachable = fleet.isSuccess;
            const status = fleet.isLoading ? "checking" : reachable ? "online" : "offline";
            return (
            <ListRow
              key={server.id}
              title={server.name}
              subtitle={`${maskToken(server.token)} · ${status} · ${rows.filter((row) => row.waiting).length} waiting`}
              onPress={() => setActive(server.id)}
              trailing={
                <IconButton
                  icon={<Trash2 size={20} strokeWidth={1.75} color={tokens.ink3} />}
                  accessibilityLabel={`Remove server ${server.name}`}
                  onPress={() => setPendingRemove(server)}
                />
              }
              hasInteractiveTrailing
              leading={
                <View style={styles.serverLeading}>
                  <View style={[styles.reachabilityDot, { backgroundColor: fleet.isLoading ? tokens.warn : reachable ? tokens.success : tokens.danger }]} />
                  {server.id === activeServerId ? <Badge label="active" tone="accent" /> : null}
                </View>
              }
            />
            );
          })}
          <ListRow
            title="Add server"
            leading={<Plus size={20} strokeWidth={1.75} color={tokens.accent} />}
            onPress={() => router.push("/connect")}
            showSeparator={false}
          />
        </Card>
      </View>

      <View>
        <SectionHeader>Connection</SectionHeader>
        <Card padded={false}>
          <ListRow
            title="Connection health"
            subtitle={healthCopy}
            leading={<Badge label={health === "checking" ? "checking" : health === "ok" ? "online" : "attention"} tone={healthTone} />}
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

      <View>
        <SectionHeader>Feedback</SectionHeader>
        <Card padded={false}>
          <ListRow
            title="Haptics"
            subtitle="Use tactile feedback for actions and status changes."
            showSeparator={false}
            trailing={<Switch value={hapticsEnabled} onValueChange={onHapticsChange} accessibilityLabel="Haptics" />}
          />
        </Card>
      </View>

      {NOTIFICATIONS_SUPPORTED ? (
        <View>
          <SectionHeader>Notifications</SectionHeader>
          <Card padded={false}>
            <ListRow
              title={isIOS ? "Push notifications" : "Web push"}
              subtitle={
                !pushSupported
                  ? "not supported in this browser."
                  : pushStatus === "subscribed"
                    ? isIOS
                      ? "Forge can notify you here when a session needs you."
                      : "Allow/Deny prompts reach you here, even with this tab closed."
                    : isIOS
                      ? "get notified when a session needs your input."
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
      ) : isTauri ? (
        <View>
          <SectionHeader>Notifications</SectionHeader>
          <Card padded={false}>
            <ListRow
              title="Desktop notifications"
              subtitle="Forge sends a native notification when a session needs you and the window isn't focused."
              trailing={<Badge label={notifyPermissionLabel(notifyPermission)} tone={notifyPermissionTone(notifyPermission)} />}
            />
            <ListRow
              title="Send test notification"
              leading={<Bell size={20} strokeWidth={1.75} color={tokens.accent} />}
              onPress={onSendTestNotification}
              disabled={notifyBusy}
              showSeparator={false}
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
          {isTauri ? <ListRow title={desktopUpdate ? `Update available: ${desktopUpdate.version}` : "Check for updates"} subtitle={desktopUpdate ? "Install and relaunch Forge" : "Desktop releases are checked in the background"} onPress={desktopUpdate ? installDesktopUpdate : checkDesktopUpdate} showSeparator={false} /> : null}
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
  serverLeading: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  reachabilityDot: { width: 8, height: 8, borderRadius: 4 },
  content: { paddingTop: space.space16, paddingBottom: space.space32, gap: space.space20 },
  pageTitle: { paddingHorizontal: space.space4 },
  appearanceCard: { gap: space.space8 },
});
