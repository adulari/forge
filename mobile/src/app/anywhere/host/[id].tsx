// Forge Anywhere — Host detail (mobile.dc.html "AW Host Detail" lines 405-464).
// `AnywhereHost` (lib/anywhere/types.ts) carries no `sessions` field, so the design's
// "sessions on this host" list is intentionally omitted rather than fabricated —
// that data would need a real per-host session index the foundation doesn't expose yet.
import { router, useLocalSearchParams } from "expo-router";
import { ServerOff } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { SettingsShell } from "../../(tabs)/settings";
import { HostDot } from "../../../components/anywhere/HostDot";
import { BackLink } from "../../../components/ds/BackLink";
import { Button } from "../../../components/ds/Button";
import { ConfirmDialog } from "../../../components/ds/ConfirmDialog";
import { EmptyState } from "../../../components/ds/EmptyState";
import { Input } from "../../../components/ds/Input";
import { Screen } from "../../../components/ds/Screen";
import { SectionHeader } from "../../../components/ds/SectionHeader";
import { Segmented } from "../../../components/ds/Segmented";
import { Sheet } from "../../../components/ds/Sheet";
import { SkeletonRow } from "../../../components/ds/Skeleton";
import { useToast } from "../../../components/ds/ToastHost";
import { goBackOr } from "../../../lib/nav";
import { hostStateText } from "../../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../../lib/anywhere/store";
import type { HostReachability, TransportPreference } from "../../../lib/anywhere/types";
import { useTokens } from "../../../theme/ThemeProvider";
import { space } from "../../../theme/tokens";
import { formatRelativeTime, type as typeScale, tabularNums } from "../../../theme/typography";

const TRANSPORT_OPTIONS: { value: TransportPreference; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "direct", label: "Direct" },
  { value: "anywhere", label: "Anywhere" },
];

const TRANSPORT_COPY =
  "Auto prefers a paired Direct connection and falls back to the relay. Direct requires explicit pairing (QR or URL) — LAN discovery can prefill, never auto-pair. Anywhere never carries the host's local daemon token. Switching transport never duplicates or leaves the session.";

function reachableViaText(via: HostReachability[]): string {
  const labels = via.map((r) => (r === "direct-lan" ? "direct · lan" : "anywhere · relay"));
  return labels.length > 0 ? labels.join(" and ") : "unreachable";
}

function formatHeartbeat(ageSec: number): string {
  return `${formatRelativeTime(Date.now() - ageSec * 1000)} ago`;
}

function DetailRow({
  label,
  value,
  valueMono = false,
  meta,
  action,
}: {
  label: string;
  value: string;
  valueMono?: boolean;
  meta?: string;
  action?: { label: string; onPress: () => void };
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.detailRow, { borderBottomColor: tokens.hairline }]}>
      <Text style={[typeScale.sub, styles.detailLabel, { color: tokens.ink3 }]} numberOfLines={1}>
        {label}
      </Text>
      <Text
        style={[
          valueMono ? typeScale.codeSmall : typeScale.sub,
          tabularNums,
          styles.detailValue,
          { color: tokens.ink },
        ]}
        numberOfLines={1}
      >
        {value}
      </Text>
      {meta ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]} numberOfLines={1}>
          {meta}
        </Text>
      ) : null}
      {action ? (
        <Pressable onPress={action.onPress} accessibilityRole="button" accessibilityLabel={action.label}>
          <Text style={[typeScale.bodyBold, { color: tokens.accent }]}>{action.label}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export default function HostDetailScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const { id } = useLocalSearchParams<{ id: string }>();
  const { signedIn, loading: accountLoading, client } = useAnywhere();
  const { hosts, loading, refresh } = useAnywhereHosts();
  const host = hosts.find((h) => h.id === id) ?? null;

  const [renameVisible, setRenameVisible] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [renaming, setRenaming] = useState(false);
  const [disabling, setDisabling] = useState(false);
  const [revokeVisible, setRevokeVisible] = useState(false);
  const [revoking, setRevoking] = useState(false);

  useEffect(() => {
    if (!accountLoading && !signedIn) router.replace("/anywhere");
  }, [accountLoading, signedIn]);

  const openRename = useCallback(() => {
    if (!host) return;
    setRenameValue(host.name);
    setRenameVisible(true);
  }, [host]);

  const onConfirmRename = useCallback(async () => {
    if (!host || !renameValue.trim()) return;
    setRenaming(true);
    try {
      await client.renameHost(host.id, renameValue.trim());
      await refresh();
      setRenameVisible(false);
      toast.show("Host renamed.", { tone: "neutral" });
    } finally {
      setRenaming(false);
    }
  }, [client, host, refresh, renameValue, toast]);

  const onTransportChange = useCallback(
    async (pref: TransportPreference) => {
      if (!host) return;
      await client.setHostTransportPreference(host.id, pref);
      await refresh();
    },
    [client, host, refresh],
  );

  const onDisable = useCallback(async () => {
    if (!host) return;
    setDisabling(true);
    try {
      await client.disableHost(host.id);
      await refresh();
      toast.show("Host disabled — its slot is free.", { tone: "neutral" });
    } finally {
      setDisabling(false);
    }
  }, [client, host, refresh, toast]);

  const onRevoke = useCallback(async () => {
    if (!host) return;
    setRevoking(true);
    try {
      await client.revokeHost(host.id);
      await refresh();
      setRevokeVisible(false);
      toast.show("Host revoked.", { tone: "neutral" });
    } finally {
      setRevoking(false);
    }
  }, [client, host, refresh, toast]);

  if (!signedIn) return null;

  return (
    <SettingsShell active="anywhere">
      <Screen scroll={!loading && !!host} contentContainerStyle={styles.content}>
        <BackLink label="Hosts" onPress={() => goBackOr("/anywhere/hosts")} />

        {loading ? (
          <View style={styles.skeletonWrap}>
            <SkeletonRow />
            <SkeletonRow />
            <SkeletonRow />
          </View>
        ) : !host ? (
          <EmptyState icon={ServerOff} message="This host isn't in your account anymore." />
        ) : (
          <>
            <View style={styles.headerRow}>
              <HostDot state={host.state} />
              <Text style={[typeScale.headingBold, styles.headerName, { color: tokens.ink }]} numberOfLines={1}>
                {host.name}
              </Text>
              <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{hostStateText(host)}</Text>
            </View>

            <View style={styles.section}>
              <DetailRow label="Name" value={host.name} action={{ label: "Rename", onPress: openRename }} />
              <DetailRow label="Identity" value={host.fingerprint} valueMono meta="unchanged by rename" />
              <DetailRow label="Connector" value={`v${host.connectorVersion}`} valueMono />
              <DetailRow label="Heartbeat" value={formatHeartbeat(host.heartbeatAgeSec)} valueMono />
              <DetailRow label="Reachable via" value={reachableViaText(host.reachableVia)} valueMono />
            </View>

            <View style={styles.section}>
              <SectionHeader>Transport for new sessions</SectionHeader>
              <Segmented options={TRANSPORT_OPTIONS} value={host.transportPreference} onChange={onTransportChange} />
              <Text style={[typeScale.meta, styles.transportCopy, { color: tokens.ink3 }]}>{TRANSPORT_COPY}</Text>
            </View>

            <View style={styles.section}>
              <Button
                label={`Forge a task on ${host.name}`}
                onPress={() => router.push("/new-session")}
                fullWidth
              />

              <Pressable
                onPress={disabling ? undefined : onDisable}
                accessibilityRole="button"
                accessibilityLabel="Disable host"
                style={styles.actionRow}
              >
                <Text style={[typeScale.body, styles.actionLabel, { color: tokens.ink2 }]}>
                  {disabling ? "Disabling…" : "Disable host"}
                </Text>
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>keeps identity, frees a slot</Text>
              </Pressable>
              <View style={[styles.separator, { backgroundColor: tokens.hairline }]} />
              <Pressable
                onPress={() => setRevokeVisible(true)}
                disabled={revoking}
                accessibilityRole="button"
                accessibilityLabel="Revoke host"
                style={styles.actionRow}
              >
                <Text style={[typeScale.body, styles.actionLabel, { color: tokens.danger }]}>
                  {revoking ? "Revoking…" : "Revoke host…"}
                </Text>
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink4 }]}>new enrollment needed</Text>
              </Pressable>
              <Text style={[typeScale.meta, styles.footerNote, { color: tokens.ink4 }]}>
                Neither removes or stops local Forge on that machine.
              </Text>
            </View>
          </>
        )}
      </Screen>

      {host ? (
        <>
          <Sheet visible={renameVisible} onClose={() => setRenameVisible(false)} accessibilityLabel="Rename host">
            <View style={styles.sheetContent}>
              <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Rename host</Text>
              <Input
                label="Name"
                value={renameValue}
                onChangeText={setRenameValue}
                autoFocus
                containerStyle={styles.sheetInput}
                accessibilityLabel="Host name"
              />
              <Button
                label={renaming ? "Renaming…" : "Save"}
                onPress={onConfirmRename}
                disabled={renaming || !renameValue.trim()}
                loading={renaming}
                fullWidth
              />
            </View>
          </Sheet>

          <ConfirmDialog
            visible={revokeVisible}
            title={`Revoke ${host.name}?`}
            message="Revokes its identity — a new enrollment is needed to reconnect this machine. Neither removes or stops local Forge on that machine."
            confirmLabel="Revoke host"
            destructive
            onConfirm={onRevoke}
            onCancel={() => setRevokeVisible(false)}
          />
        </>
      ) : null}
    </SettingsShell>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space16, paddingBottom: space.space48 },
  skeletonWrap: { marginTop: space.space16, gap: space.space8 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space4, marginTop: space.space4 },
  headerName: { flex: 1 },
  section: { marginTop: space.space20 },
  detailRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: space.space8,
    paddingHorizontal: space.space4,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  detailLabel: { width: 104, flexShrink: 0 },
  detailValue: { flex: 1 },
  transportCopy: { marginTop: space.space8, lineHeight: 16 },
  actionRow: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: space.space12, marginTop: space.space8 },
  actionLabel: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth },
  footerNote: { marginTop: space.space8, lineHeight: 16 },
  sheetContent: { padding: space.space20, gap: space.space16 },
  sheetInput: { marginTop: space.space4 },
});
