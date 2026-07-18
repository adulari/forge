// Forge Anywhere — Settings-entered status hub (mobile.dc.html "AW Settings Hub" lines
// 26-91) with a desktop two-column layout at the expanded breakpoint (desktop.dc.html
// "AW Desktop Settings" lines 92-162). Signed-out renders only the explainer + sign-in,
// per the design's "First use -> the hub shows only 'Sign in with GitHub' + what Anywhere
// adds" application state (mobile.dc.html line 1347) — this matches AnywhereProvider's
// signed-out-by-default posture (store.tsx), so a fresh install never probes the client.
import { router } from "expo-router";
import React, { useCallback, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { SettingsShell } from "../(tabs)/settings";
import { EntitlementBadge } from "../../components/anywhere/EntitlementBadge";
import { BackLink } from "../../components/ds/BackLink";
import { Button } from "../../components/ds/Button";
import { ConfirmDialog } from "../../components/ds/ConfirmDialog";
import { KeyValueRow } from "../../components/ds/KeyValueRow";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Skeleton } from "../../components/ds/Skeleton";
import { useToast } from "../../components/ds/ToastHost";
import { goBackOr } from "../../lib/nav";
import { formatBytes, hostStateText } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereDevices, useAnywhereHosts, useAnywhereStorage } from "../../lib/anywhere/store";
import { MAX_ACTIVE_HOSTS, type AnywhereAccount, type AnywhereHost } from "../../lib/anywhere/types";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { formatRelativeTime, type, tabularNums } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

const PRIVACY_COPY =
  "Only your paired devices. The relay routes sealed envelopes — it sees sizes, timestamps and kinds, never contents. If every device and your recovery phrase are lost, no one can decrypt this account, including Forge support.";
const LOCAL_FALLBACK_NOTE = "Local Forge keeps working without Anywhere — Direct and LAN access stay free.";

function relaySubtitle(account: AnywhereAccount): string {
  if (!account.relayConnected) return "Relay disconnected";
  if (account.lastSyncAt == null) return "Relay connected · not synced yet";
  return `Relay connected · last sync ${formatRelativeTime(account.lastSyncAt)} ago`;
}

/** Trial copy mirrors the design's exact business-rule state machine (types.ts lines 12-15). */
function billingMeta(account: AnywhereAccount): string {
  switch (account.entitlement) {
    case "not-started":
      return "not started";
    case "trial":
      return account.trialDaysLeft != null ? `trial · ${account.trialDaysLeft} days left` : "trial";
    case "active":
      return `active · ${account.plan ?? "monthly"}`;
    case "grace":
      return account.graceDaysLeft != null ? `grace · ${account.graceDaysLeft} days left` : "grace";
    case "read-only":
      return account.readOnlyDaysLeft != null ? `read-only · ${account.readOnlyDaysLeft} days left` : "read-only";
    case "suspended":
      return account.deletesInDays != null ? `suspended · deletes in ${account.deletesInDays}d` : "suspended";
    case "webhook-pending":
      return "pending";
    default: {
      const _exhaustive: never = account.entitlement;
      return _exhaustive;
    }
  }
}

function billingParagraph(account: AnywhereAccount): string {
  switch (account.entitlement) {
    case "trial":
      return `Trial · ${account.trialDaysLeft ?? 14} days left · no card on file. Annual €79/yr (default) or €10/mo via Paddle. At expiry: read-only immediately — no grace. Grace (7d) applies only to paid payment failure. Checkout restores access instantly.`;
    case "active":
      return `Active · ${account.plan === "yearly" ? "€79/yr" : "€10/mo"} via Paddle.`;
    case "grace":
      return `Payment failed · ${account.graceDaysLeft ?? 7} days of grace left before read-only.`;
    case "read-only":
      return `Read-only · ${account.readOnlyDaysLeft ?? 30} days left to restore access before suspension.`;
    case "suspended":
      return `Suspended · deletes in ${account.deletesInDays ?? 90} days unless restored.`;
    case "not-started":
      return "Not started — connect a host to begin your 14-day trial, no card required.";
    case "webhook-pending":
      return "Payment received — waiting for confirmation.";
    default: {
      const _exhaustive: never = account.entitlement;
      return _exhaustive;
    }
  }
}

function hostsSummary(hosts: AnywhereHost[]): string {
  const active = hosts.filter((h) => h.state.kind !== "revoked" && h.state.kind !== "disabled");
  const anyOnline = active.some((h) => h.state.kind === "online");
  const status = anyOnline ? "online" : active.length === 0 ? "none active" : "offline";
  return `${active.length} of ${MAX_ACTIVE_HOSTS} · ${status}`;
}

function SignedOutBody() {
  const tokens = useTokens();
  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Settings" onPress={() => goBackOr("/settings")} />
      <Text style={[type.title, styles.pageTitle, { color: tokens.ink }]}>Forge Anywhere</Text>

      <View style={styles.explainerBlock}>
        <Text style={[type.bodyBold, { color: tokens.ink }]}>What Forge Anywhere adds</Text>
        <Text style={[type.sub, styles.explainerLine, { color: tokens.ink2 }]}>
          Reach your hosts from anywhere, not just your LAN. Pair extra devices to control the same sessions.
          Sessions, settings and memories sync end-to-end encrypted — the relay only ever sees sealed envelopes.
        </Text>
      </View>

      <Button
        label="Sign in with GitHub"
        onPress={() => router.push("/anywhere/sign-in")}
        fullWidth
        style={styles.signInButton}
      />

      <Text style={[type.meta, styles.footerNote, { color: tokens.ink4 }]}>{LOCAL_FALLBACK_NOTE}</Text>
    </Screen>
  );
}

function HubSkeleton() {
  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Settings" onPress={() => goBackOr("/settings")} />
      <Skeleton width={180} height={22} style={styles.skeletonTitle} />
      <Skeleton width={220} height={13} style={styles.skeletonSubtitle} />
      <View style={styles.skeletonRows}>
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <Skeleton key={i} width="100%" height={44} />
        ))}
      </View>
    </Screen>
  );
}

function CompactHub({ account }: { account: AnywhereAccount }) {
  const tokens = useTokens();
  const toast = useToast();
  const { hosts, loading: hostsLoading } = useAnywhereHosts();
  const { devices, loading: devicesLoading } = useAnywhereDevices();
  const { storage, loading: storageLoading } = useAnywhereStorage();

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Settings" onPress={() => goBackOr("/settings")} />

      <View style={styles.titleRow}>
        <Text style={[type.title, styles.titleFlex, { color: tokens.ink }]}>Forge Anywhere</Text>
        <Pressable
          onPress={() => router.push("/anywhere/billing")}
          accessibilityRole="button"
          accessibilityLabel="Billing"
        >
          <EntitlementBadge account={account} />
        </Pressable>
      </View>
      <Text style={[type.sub, styles.subtitle, { color: tokens.ink3 }]}>{relaySubtitle(account)}</Text>

      <View style={styles.section}>
        <SectionHeader>Account</SectionHeader>
        <KeyValueRow label={`@${account.githubLogin}`} value="github" chevron onPress={() => router.push("/anywhere/account")} />
        <KeyValueRow
          label="Hosts"
          value={hostsLoading ? undefined : hostsSummary(hosts)}
          chevron
          onPress={() => router.push("/anywhere/hosts")}
        />
        <KeyValueRow
          label="Devices"
          value={devicesLoading ? undefined : `${devices.length} paired`}
          chevron
          onPress={() => router.push("/anywhere/devices")}
        />
        <KeyValueRow label="Notifications" value="on" chevron onPress={() => router.push("/anywhere/notifications")} />
        <KeyValueRow
          label="Encrypted storage"
          value={storageLoading || !storage ? undefined : `${formatBytes(storage.usedBytes)} of ${formatBytes(storage.quotaBytes)}`}
          chevron
          onPress={() => router.push("/anywhere/storage")}
        />
        <KeyValueRow label="Billing" value={billingMeta(account)} chevron onPress={() => router.push("/anywhere/billing")} />
      </View>

      <View style={styles.section}>
        <SectionHeader>Who can read your data</SectionHeader>
        <Text style={[type.sub, styles.privacyText, { color: tokens.ink2 }]}>{PRIVACY_COPY}</Text>
        <Pressable
          onPress={() => toast.show("Technical disclosure — coming soon.", { tone: "neutral" })}
          accessibilityRole="button"
          accessibilityLabel="Technical disclosure"
        >
          <Text style={[type.bodyBold, styles.disclosureLink, { color: tokens.accent }]}>Technical disclosure</Text>
        </Pressable>
      </View>

      <Text style={[type.meta, styles.footerNote, { color: tokens.ink4 }]}>{LOCAL_FALLBACK_NOTE}</Text>
    </Screen>
  );
}

function WideHub({ account }: { account: AnywhereAccount }) {
  const tokens = useTokens();
  const toast = useToast();
  const { client, refresh } = useAnywhere();
  const { hosts, loading: hostsLoading } = useAnywhereHosts();
  const { devices, loading: devicesLoading } = useAnywhereDevices();
  const { storage, loading: storageLoading } = useAnywhereStorage();
  const [deleteConfirmVisible, setDeleteConfirmVisible] = useState(false);
  const [exporting, setExporting] = useState(false);

  const onExport = useCallback(async () => {
    setExporting(true);
    try {
      await client.exportAccountData();
      toast.show("Account data exported.", { tone: "neutral" });
    } finally {
      setExporting(false);
    }
  }, [client, toast]);

  const onSignOut = useCallback(async () => {
    await client.signOut();
    await refresh();
    toast.show("Signed out of Forge Anywhere.", { tone: "neutral" });
  }, [client, refresh, toast]);

  const onDeleteAccount = useCallback(async () => {
    setDeleteConfirmVisible(false);
    await client.deleteAccount();
    await refresh();
    toast.show("Account deleted.", { tone: "neutral" });
  }, [client, refresh, toast]);

  const pct = storage && storage.quotaBytes > 0 ? Math.round((storage.usedBytes / storage.quotaBytes) * 100) : 0;

  return (
    <Screen scroll contentContainerStyle={styles.wideContent}>
      <View style={styles.wideHeaderRow}>
        <Text style={[type.title, { color: tokens.ink }]}>Forge Anywhere</Text>
        <Pressable onPress={() => router.push("/anywhere/billing")} accessibilityRole="button" accessibilityLabel="Billing">
          <EntitlementBadge account={account} />
        </Pressable>
      </View>
      <Text style={[type.sub, styles.subtitle, { color: tokens.ink3 }]}>
        {`@${account.githubLogin} · ${relaySubtitle(account).toLowerCase()}`}
      </Text>

      <View style={styles.wideColumns}>
        <View style={styles.wideLeft}>
          <SectionHeader>{`Hosts · ${hostsLoading ? "…" : hostsSummary(hosts)}`}</SectionHeader>
          {hostsLoading ? (
            <Skeleton width="100%" height={44} />
          ) : (
            hosts.map((host) => (
              <KeyValueRow
                key={host.id}
                label={host.name}
                value={hostStateText(host)}
                chevron
                onPress={() => router.push({ pathname: "/anywhere/host/[id]", params: { id: host.id } })}
              />
            ))
          )}
          <Pressable
            onPress={() => toast.show("Copy the command from the Hosts screen.", { tone: "neutral" })}
            accessibilityRole="button"
            accessibilityLabel="Add a host"
            style={[styles.addHostBox, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
          >
            <Text style={[type.codeSmall, tabularNums, styles.addHostText, { color: tokens.ink }]} numberOfLines={1}>
              <Text style={{ color: tokens.ink3 }}>$ </Text>
              forge anywhere enable --name NAME
            </Text>
          </Pressable>

          <SectionHeader>{`Devices · ${devicesLoading ? "…" : devices.length}`}</SectionHeader>
          {devicesLoading ? (
            <Skeleton width="100%" height={44} />
          ) : (
            devices.map((device) => (
              <KeyValueRow
                key={device.id}
                label={device.isThisDevice ? `${device.name} · this device` : device.name}
                value={device.isThisDevice ? `enrolled ${formatRelativeTime(device.enrolledAt)} ago` : `last seen ${formatRelativeTime(device.lastSeenAt)} ago`}
                onPress={() => router.push("/anywhere/devices")}
              />
            ))
          )}
          <View style={styles.wideLinkRow}>
            <Pressable onPress={() => router.push("/anywhere/pair")} accessibilityRole="button" accessibilityLabel="Pair a device">
              <Text style={[type.bodyBold, { color: tokens.accent }]}>Pair a device</Text>
            </Pressable>
            <Pressable onPress={() => router.push("/anywhere/devices")} accessibilityRole="button" accessibilityLabel="Lost a device?">
              <Text style={[type.bodyBold, { color: tokens.danger }]}>Lost a device?</Text>
            </Pressable>
          </View>
        </View>

        <View style={styles.wideRight}>
          <SectionHeader>Storage</SectionHeader>
          {storageLoading || !storage ? (
            <Skeleton width="100%" height={40} />
          ) : (
            <>
              <View style={styles.storageFigureRow}>
                <Text style={[type.codeSmall, tabularNums, styles.storageFigure, { color: tokens.ink }]}>
                  {formatBytes(storage.usedBytes)}
                </Text>
                <Text style={[type.monoMeta, tabularNums, { color: tokens.ink3 }]}>{`of ${formatBytes(storage.quotaBytes)} · ${pct}%`}</Text>
              </View>
              <View style={[styles.gaugeTrack, { backgroundColor: tokens.border }]}>
                <View style={[styles.gaugeFill, { width: `${Math.min(100, pct)}%`, backgroundColor: tokens.accent }]} />
              </View>
            </>
          )}

          <SectionHeader style={styles.wideSectionSpacing}>Billing</SectionHeader>
          <Text style={[type.sub, styles.billingText, { color: tokens.ink2 }]}>{billingParagraph(account)}</Text>
          <Button label="Choose a plan" onPress={() => router.push("/anywhere/billing")} style={styles.chooseplanButton} />

          <SectionHeader style={styles.wideSectionSpacing}>Notifications</SectionHeader>
          <KeyValueRow label="Generic pushes" value="enabled" onPress={() => router.push("/anywhere/notifications")} />

          <SectionHeader style={styles.wideSectionSpacing}>Account</SectionHeader>
          <Pressable onPress={onExport} disabled={exporting} accessibilityRole="button" accessibilityLabel="Export account data">
            <Text style={[type.body, styles.accountLink, { color: tokens.ink2 }]}>
              {exporting ? "Exporting…" : "Export account data"}
            </Text>
          </Pressable>
          <Pressable onPress={onSignOut} accessibilityRole="button" accessibilityLabel="Sign out on this device">
            <Text style={[type.body, styles.accountLink, { color: tokens.ink2 }]}>Sign out on this device</Text>
          </Pressable>
          <Pressable
            onPress={() => setDeleteConfirmVisible(true)}
            accessibilityRole="button"
            accessibilityLabel="Delete account"
          >
            <Text style={[type.body, styles.accountLink, { color: tokens.danger }]}>Delete account…</Text>
          </Pressable>

          <Text style={[type.meta, styles.wideFooterNote, { color: tokens.ink4 }]}>
            Direct servers, appearance, app lock, usage and diagnostics stay in their existing Settings sections.
          </Text>
        </View>
      </View>

      <ConfirmDialog
        visible={deleteConfirmVisible}
        title="Delete your Forge Anywhere account?"
        message="This deletes your encrypted account, hosts, devices and stored data after the retention window. Local Forge on every machine keeps working."
        confirmLabel="Delete account"
        destructive
        onConfirm={onDeleteAccount}
        onCancel={() => setDeleteConfirmVisible(false)}
      />
    </Screen>
  );
}

export default function AnywhereHubScreen() {
  const { account, loading, signedIn } = useAnywhere();
  const { isExpanded } = useBreakpoint();

  return (
    <SettingsShell active="anywhere">
      {loading ? (
        <HubSkeleton />
      ) : !signedIn || !account ? (
        <SignedOutBody />
      ) : isExpanded ? (
        <WideHub account={account} />
      ) : (
        <CompactHub account={account} />
      )}
    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48, gap: space.space4 },
  pageTitle: { paddingHorizontal: space.space4, marginTop: space.space4 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space4, marginTop: space.space4 },
  titleFlex: { flex: 1 },
  subtitle: { paddingHorizontal: space.space4, marginTop: -2 },
  explainerBlock: { marginTop: space.space24, paddingHorizontal: space.space4, gap: space.space8 },
  explainerLine: { lineHeight: 19 },
  signInButton: { marginTop: space.space24, marginHorizontal: space.space4 },
  footerNote: { marginTop: space.space24, paddingHorizontal: space.space4 },
  skeletonTitle: { marginTop: space.space12, marginHorizontal: space.space4 },
  skeletonSubtitle: { marginTop: space.space8, marginHorizontal: space.space4 },
  skeletonRows: { marginTop: space.space24, gap: space.space8 },
  section: { marginTop: space.space16 },
  privacyText: { paddingHorizontal: space.space16, marginTop: space.space8, lineHeight: 19 },
  disclosureLink: { paddingHorizontal: space.space16, marginTop: space.space8, paddingVertical: space.space4 },
  wideContent: { paddingTop: space.space24, paddingBottom: space.space48, maxWidth: 720, alignSelf: "center", width: "100%" },
  wideHeaderRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  wideColumns: { flexDirection: "row", gap: 44, marginTop: space.space24 },
  wideLeft: { flex: 1, minWidth: 0 },
  wideRight: { width: 290, flexShrink: 0 },
  wideSectionSpacing: { marginTop: space.space16 },
  addHostBox: { marginTop: space.space12, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius12, paddingHorizontal: space.space12, paddingVertical: space.space12 },
  addHostText: { flexShrink: 1 },
  wideLinkRow: { flexDirection: "row", gap: space.space16, marginTop: space.space8 },
  storageFigureRow: { flexDirection: "row", alignItems: "baseline", gap: space.space8, marginTop: space.space8 },
  storageFigure: { fontSize: 19, fontWeight: "700" },
  gaugeTrack: { height: 4, borderRadius: 2, overflow: "hidden", marginTop: space.space8 },
  gaugeFill: { height: "100%", borderRadius: 2 },
  billingText: { marginTop: space.space8, lineHeight: 19 },
  chooseplanButton: { marginTop: space.space12 },
  accountLink: { paddingVertical: space.space4, marginTop: space.space8 },
  wideFooterNote: { marginTop: space.space20, lineHeight: 17 },
});
