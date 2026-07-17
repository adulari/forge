// Settings (FEATURES.md §4 IA, BUILD_ORDER.md T2.2; Hearth redesign). De-boxed
// type-first hairline rows (core rule 1) — no Card wrapping around lists, the
// decision card stays reserved for pending permissions/plans elsewhere.
//
// Server removal is via the trailing delete IconButton + ConfirmDialog
// (ds/ListRow has no swipe-action variant yet — that's a fleet/SessionCard-
// level concern per BUILD_ORDER T2.3, out of this task's scope).
//
// The app-lock preference key is `forge.appLock` (AsyncStorage, boolean "true"/"false") —
// `src/components/AppLock.tsx` reads/writes the same key so the Settings toggle and the
// lock gate agree.
//
// `SettingsNavRail`/`SettingsShell` (exported below) give every settings sub-page
// (usage/models/plans/mcp/configuration/skills/hooks/session-tree) the desktop 240px
// nav rail from the Hearth desktop prototype. They render inside this route file
// (rather than a new shared component) to stay within this builder's file scope —
// sibling route files import the named exports.
import AsyncStorage from "@react-native-async-storage/async-storage";
import Constants from "expo-constants";
import { router } from "expo-router";
import { Bell, ChevronRight, Plus, Trash2 } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Badge, type BadgeTone } from "../../components/ds/Badge";
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
import {
  isAnonymousTelemetryEnabled,
  setAnonymousTelemetryEnabled,
} from "../../lib/anonymousTelemetry";
import { useHooks, useMcp, useModels, usePlans, useServerFleets, useSkills } from "../../lib/queries";
import { isIOS, isTauri, isWeb } from "../../lib/platform";
import { checkForDesktopUpdate, type DesktopUpdate } from "../../lib/updater";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type, tabularNums } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

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

// -----------------------------------------------------------------------------
// Desktop 240px Settings nav rail (Hearth desktop prototype, "Desktop Settings").
// Used by every settings sub-page at the `expanded` breakpoint.
// -----------------------------------------------------------------------------

type SettingsRoute = "/settings" | "/usage" | "/models" | "/plans" | "/mcp" | "/configuration" | "/skills" | "/hooks" | "/session-tree";

const SETTINGS_NAV_ITEMS: { key: string; label: string; href: SettingsRoute }[] = [
  { key: "general", label: "General", href: "/settings" },
  { key: "usage", label: "Usage", href: "/usage" },
  { key: "models", label: "Models & mesh", href: "/models" },
  { key: "plans", label: "Plans", href: "/plans" },
  { key: "mcp", label: "MCP servers", href: "/mcp" },
  { key: "configuration", label: "Configuration", href: "/configuration" },
  { key: "skills", label: "Skills", href: "/skills" },
  { key: "hooks", label: "Hooks", href: "/hooks" },
  { key: "session-tree", label: "Session tree", href: "/session-tree" },
];

export function SettingsNavRail({ active }: { active: string }) {
  const tokens = useTokens();
  const appVersion = Constants.expoConfig?.version ?? "—";
  return (
    <View style={[railStyles.rail, { borderRightColor: tokens.border }]}>
      <Text style={[type.headingBold, railStyles.title, { color: tokens.ink }]}>Settings</Text>
      {SETTINGS_NAV_ITEMS.map((item) => {
        const selected = item.key === active;
        return (
          <Pressable
            key={item.key}
            onPress={() => router.push(item.href)}
            accessibilityRole="button"
            accessibilityLabel={item.label}
            accessibilityState={{ selected }}
            style={[railStyles.item, selected ? { backgroundColor: tokens.selection } : null]}
          >
            <Text style={[type.bodyBold, { color: selected ? tokens.accent : tokens.ink2 }]} numberOfLines={1}>
              {item.label}
            </Text>
          </Pressable>
        );
      })}
      <View style={railStyles.flexFill} />
      <Text style={[type.monoMeta, tabularNums, railStyles.version, { color: tokens.ink4 }]}>{`v${appVersion} · protocol v7`}</Text>
    </View>
  );
}

/** Wraps a settings sub-page with the desktop nav rail at `expanded`; a no-op elsewhere. */
export function SettingsShell({ active, children }: { active: string; children: React.ReactNode }) {
  const { isExpanded } = useBreakpoint();
  if (!isExpanded) return <>{children}</>;
  return (
    <View style={railStyles.shellRow}>
      <SettingsNavRail active={active} />
      <View style={railStyles.pane}>{children}</View>
    </View>
  );
}

function NavListRow({ label, meta, onPress, showSeparator = true }: { label: string; meta?: string; onPress: () => void; showSeparator?: boolean }) {
  const tokens = useTokens();
  return (
    <ListRow
      title={label}
      onPress={onPress}
      showSeparator={showSeparator}
      trailing={
        <View style={styles.navTrailing}>
          {meta ? <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]}>{meta}</Text> : null}
          <ChevronRight size={15} strokeWidth={1.75} color={tokens.ink4} />
        </View>
      }
    />
  );
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
  const [anonymousTelemetry, setAnonymousTelemetry] = useState(true);
  const [anonymousTelemetryLoaded, setAnonymousTelemetryLoaded] = useState(false);
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

  // "forge" nav row trailing counts — real data from the same hooks each sub-page
  // uses (react-query dedupes by baseUrl-scoped key, so this doesn't add a
  // duplicate network round-trip beyond the shared cache).
  const modelsQuery = useModels();
  const mcpQuery = useMcp();
  const plansQuery = usePlans();
  const skillsQuery = useSkills();
  const hooksQuery = useHooks();
  const modelsReadyLabel = useMemo(() => {
    if (!modelsQuery.data) return undefined;
    const ready = modelsQuery.data.providers.flatMap((p) => p.models).filter((m) => m.health == null).length;
    return `${ready} ready`;
  }, [modelsQuery.data]);
  const plansOpenLabel = plansQuery.data ? `${plansQuery.data.length} open` : undefined;
  const mcpEnabledLabel = mcpQuery.data ? `${mcpQuery.data.servers.filter((s) => s.enabled).length} enabled` : undefined;
  const skillsCountLabel = skillsQuery.data ? `${skillsQuery.data.length}` : undefined;
  const hooksCountLabel = hooksQuery.data ? `${hooksQuery.data.length}` : undefined;

  useEffect(() => {
    let cancelled = false;
    void isAnonymousTelemetryEnabled().then((enabled) => {
      if (!cancelled) {
        setAnonymousTelemetry(enabled);
        setAnonymousTelemetryLoaded(true);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

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

  const healthStatusWord = health === "ok" ? "online" : health === "checking" ? "checking…" : health === "bad-token" ? "pairing invalid" : health === "unreachable" ? "offline" : health === "server-error" ? "server error" : "not connected";
  const healthDotColor = health === "ok" ? tokens.success : health === "checking" ? tokens.warn : health === "idle" ? tokens.ink3 : tokens.danger;
  const healthMetaText = host ? `${healthStatusWord} · protocol v7 · ${host}` : healthStatusWord;

  const onAppLockChange = (value: boolean) => {
    const previous = appLock;
    setAppLock(value);
    void AsyncStorage.setItem(APP_LOCK_KEY, value ? "true" : "false").catch(() => {
      setAppLock(previous);
      toast.show("couldn't save app lock preference.", { tone: "danger" });
    });
  };

  const onHapticsChange = (value: boolean) => {
    setHapticsEnabledState(value);
    setHapticsEnabled(value);
    if (value) haptics.sendPrompt();
  };

  const onAnonymousTelemetryChange = (value: boolean) => {
    const previous = anonymousTelemetry;
    setAnonymousTelemetry(value);
    void setAnonymousTelemetryEnabled(value).catch(() => {
      setAnonymousTelemetry(previous);
      toast.show("couldn't save anonymous statistics preference.", { tone: "danger" });
    });
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
    <SettingsShell active="general">
      <Screen scroll contentContainerStyle={styles.content}>
        <Text style={[type.title, styles.pageTitle, { color: tokens.ink }]}>Settings</Text>

        <View>
          <SectionHeader>Servers</SectionHeader>
          {servers.map((server, index) => {
            const fleet = serverQueries[index];
            const rows = fleet.data ?? [];
            const reachable = fleet.isSuccess;
            const waitingCount = rows.filter((row) => row.waiting).length;
            return (
              <ListRow
                key={server.id}
                title={server.name}
                onPress={() => setActive(server.id)}
                leading={
                  <View style={styles.serverLeading}>
                    <View style={[styles.reachabilityDot, { backgroundColor: fleet.isLoading ? tokens.warn : reachable ? tokens.success : tokens.danger }]} />
                    {server.id === activeServerId ? <Badge label="active" tone="accent" /> : null}
                  </View>
                }
                trailing={
                  <View style={styles.serverTrailing}>
                    <Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>{`${maskToken(server.token)} · ${waitingCount} waiting`}</Text>
                    <IconButton
                      icon={<Trash2 size={18} strokeWidth={1.75} color={tokens.ink3} />}
                      accessibilityLabel={`Remove server ${server.name}`}
                      onPress={() => setPendingRemove(server)}
                    />
                  </View>
                }
                hasInteractiveTrailing
              />
            );
          })}
          <ListRow
            title="Add server"
            leading={<Plus size={18} strokeWidth={1.75} color={tokens.accent} />}
            onPress={() => router.push("/connect")}
            showSeparator={false}
          />
        </View>

        <View>
          <SectionHeader>Forge</SectionHeader>
          <NavListRow label="Usage" onPress={() => router.push("/usage")} />
          <NavListRow label="Models & mesh health" meta={modelsReadyLabel} onPress={() => router.push("/models")} />
          <NavListRow label="Plans" meta={plansOpenLabel} onPress={() => router.push("/plans")} />
          <NavListRow label="MCP servers" meta={mcpEnabledLabel} onPress={() => router.push("/mcp")} />
          <NavListRow label="Configuration" onPress={() => router.push("/configuration")} />
          <NavListRow label="Skills" meta={skillsCountLabel} onPress={() => router.push("/skills")} />
          <NavListRow label="Hooks" meta={hooksCountLabel} onPress={() => router.push("/hooks")} />
          <NavListRow label="Session tree" onPress={() => router.push("/session-tree")} showSeparator={false} />
        </View>

        <View>
          <SectionHeader>Appearance</SectionHeader>
          <Segmented
            options={[
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
              { value: "system", label: "System" },
            ]}
            value={preference}
            onChange={setScheme}
          />
        </View>

        <View>
          <SectionHeader>Connection</SectionHeader>
          <ListRow
            title="Connection health"
            leading={<View style={[styles.reachabilityDot, { backgroundColor: healthDotColor }]} />}
            trailing={<Text style={[type.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>{healthMetaText}</Text>}
            showSeparator={false}
          />
        </View>

        <View>
          <SectionHeader>Security</SectionHeader>
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
        </View>

        <View>
          <SectionHeader>Privacy</SectionHeader>
          <ListRow
            title="Anonymous usage statistics"
            subtitle="Share install and activity counts. Never sends code, prompts, paths, accounts, or a device ID."
            showSeparator={false}
            trailing={
              anonymousTelemetryLoaded ? (
                <Switch
                  value={anonymousTelemetry}
                  onValueChange={onAnonymousTelemetryChange}
                  accessibilityLabel="Anonymous usage statistics"
                />
              ) : undefined
            }
          />
        </View>

        <View>
          <SectionHeader>Feedback</SectionHeader>
          <ListRow
            title="Haptics"
            subtitle="Use tactile feedback for actions and status changes."
            showSeparator={false}
            trailing={<Switch value={hapticsEnabled} onValueChange={onHapticsChange} accessibilityLabel="Haptics" />}
          />
        </View>

        {NOTIFICATIONS_SUPPORTED ? (
          <View>
            <SectionHeader>Notifications</SectionHeader>
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
          </View>
        ) : isTauri ? (
          <View>
            <SectionHeader>Notifications</SectionHeader>
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
          </View>
        ) : null}

        <View>
          <SectionHeader>About &amp; diagnostics</SectionHeader>
          <KeyValueRow label="Version" value={appVersion} />
          <KeyValueRow label="Protocol" value="v7" />
          <KeyValueRow label="Active server" value={host ? `${host} · ${maskToken(activeToken)}` : "none"} />
          {isTauri ? <ListRow title={updateBusy ? "Checking for updates…" : desktopUpdate ? `Update available: ${desktopUpdate.version}` : "Check for updates"} subtitle={desktopUpdate ? "Install and relaunch Forge" : "Desktop releases are checked in the background"} onPress={updateBusy ? undefined : desktopUpdate ? installDesktopUpdate : checkDesktopUpdate} showSeparator={false} /> : null}
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
    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  serverLeading: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  serverTrailing: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  navTrailing: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  reachabilityDot: { width: 7, height: 7, borderRadius: 4 },
  content: { paddingTop: space.space16, paddingBottom: space.space48, gap: space.space20 },
  pageTitle: { paddingHorizontal: space.space4 },
});

const railStyles = StyleSheet.create({
  shellRow: { flex: 1, flexDirection: "row", minHeight: 0 },
  rail: { width: 240, flexShrink: 0, borderRightWidth: StyleSheet.hairlineWidth, paddingHorizontal: space.space12, paddingVertical: space.space16, gap: 2 },
  title: { paddingHorizontal: space.space8, paddingBottom: space.space12 },
  item: { minHeight: 36, paddingHorizontal: space.space8, borderRadius: 8, justifyContent: "center" },
  flexFill: { flex: 1 },
  version: { paddingHorizontal: space.space8 },
  pane: { flex: 1, minWidth: 0 },
});
