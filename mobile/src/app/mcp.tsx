// Machined MCP servers ("D Settings Plans MCP Config" L869-891 / "M Config MCP Plans" L384-401):
// dense mono rows with a live enable/disable Switch driven by `PATCH /api/mcp`.
//
// The daemon only owns the two `mcp.toml` files the CLI writes, so a server that resolves from an
// imported `.mcp.json` is refused with a 404 rather than being forked into a new `mcp.toml` entry.
// `McpServerRow` carries no "which file defines this" field, so that case is undetectable until
// the first PATCH comes back — the row is then latched read-only for the rest of the session with
// the daemon's own explanation, instead of leaving a toggle that can only ever fail.
import React, { useCallback, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { AddMcpServerSheet } from "../components/mcp/AddMcpServerSheet";
import { Badge } from "../components/ds/Badge";
import { BackLink } from "../components/ds/BackLink";
import { Button } from "../components/ds/Button";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { Skeleton } from "../components/ds/Skeleton";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { ApiError } from "../lib/api";
import { useMcp, useUpdateMcpServer } from "../lib/queries";
import { Plug } from "lucide-react-native";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

/** The daemon's own refusal text. `request()` rewrites every 404's message to the pairing-invalid
 * copy (it cannot tell a bad token from a real 404), so the true reason survives only in `body`. */
function externalConfigReason(error: unknown): string | null {
  if (!(error instanceof ApiError) || error.status !== 404) return null;
  const body = error.body as { error?: string } | undefined;
  return body?.error ?? "not defined in a mcp.toml Forge writes — edit it where it is defined";
}

// Shape-matched loading placeholder for one server row — without this, `query.isLoading` fell
// through to `data?.servers.map` on `undefined` data (no branch at all, not even the "Pull to
// retry" text the other two settings-family screens at least had) and rendered nothing below
// the title/subtitle for the whole load.
function McpServerSkeleton() {
  const tokens = useTokens();
  return (
    <View style={styles.server}>
      <View style={[styles.dot, { backgroundColor: tokens.bg3 }]} />
      <View style={styles.serverBody}>
        <Skeleton width="40%" height={14} />
        <Skeleton width="65%" height={12} />
      </View>
    </View>
  );
}

function McpScreenBody() {
  const tokens = useTokens();
  const toast = useToast();
  const query = useMcp();
  const update = useUpdateMcpServer();
  const data = query.data;
  const [adding, setAdding] = useState(false);
  // Desired-state overlay keyed by server name. The mutation seeds the cache with the daemon's
  // full refreshed list, so the overlay is dropped on settle — success keeps the new value,
  // failure rolls back to whatever the server still reports.
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [external, setExternal] = useState<Record<string, string>>({});

  const toggle = useCallback(
    (name: string, enabled: boolean) => {
      setPending((prev) => ({ ...prev, [name]: enabled }));
      update.mutate(
        { name, enabled },
        {
          onSettled: () =>
            setPending((prev) => {
              const next = { ...prev };
              delete next[name];
              return next;
            }),
          onError: (error) => {
            const reason = externalConfigReason(error);
            if (reason) {
              setExternal((prev) => ({ ...prev, [name]: reason }));
              toast.show(`${name} is configured elsewhere — ${reason}`, { tone: "danger" });
              return;
            }
            toast.show(
              error instanceof ApiError ? error.message : `could not ${enabled ? "enable" : "disable"} ${name}`,
              { tone: "danger" },
            );
          },
        },
      );
    },
    [toast, update],
  );

  return <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
    <View style={styles.headerRow}><BackLink /><View style={styles.flexFill} /><Pressable onPress={() => setAdding(true)} accessibilityRole="button"><Text style={[styles.add, { color: tokens.accent }]}>+ Add</Text></Pressable></View>
    <Text style={[type.title, { color: tokens.ink }]}>MCP servers</Text>
    <Text style={[type.sub, { color: tokens.ink3 }]}>External tools available to Forge. Secrets remain on the host.</Text>
    {query.isLoading ? (
      <View>
        {[0, 1, 2].map((i) => (
          <McpServerSkeleton key={i} />
        ))}
      </View>
    ) : null}
    {query.isError && !data ? (
      <EmptyState
        icon={Plug}
        message="Could not load MCP servers."
        action={<Button label="Retry" variant="secondary" onPress={() => void query.refetch()} accessibilityLabel="Retry loading MCP servers" />}
      />
    ) : null}
    {!query.isLoading && data?.servers.length === 0 ? <EmptyState icon={Plug} message="No MCP servers configured." /> : null}
    {data?.servers.map((server, index) => {
      const enabled = pending[server.name] ?? server.enabled;
      // The daemon now says up front whether it can write this server (v9); the latched 404
      // reason stays as the fallback for a pre-v9 daemon that omits the flag.
      const readOnlyReason =
        external[server.name] ??
        (server.editable === false ? "defined outside mcp.toml — edit it where it is defined" : null);
      return <View key={server.name} style={[styles.server, index < data.servers.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
        <View style={[styles.dot, { backgroundColor: enabled ? tokens.success : tokens.ink4 }]} />
        <View style={styles.serverBody}>
          <Text style={[styles.name, { color: enabled ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{server.name}</Text>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>{[server.transport, server.auth_configured ? "auth configured" : null, server.secret_env_count > 0 ? `${server.secret_env_count} secret ref${server.secret_env_count === 1 ? "" : "s"}` : null].filter(Boolean).join(" · ")}</Text>
          {readOnlyReason ? <Text style={[type.monoMeta, { color: tokens.warnBgInk }]}>{readOnlyReason}</Text> : null}
        </View>
        {readOnlyReason ? (
          <Badge label={enabled ? "enabled" : "disabled"} tone={enabled ? "success" : "neutral"} />
        ) : (
          <Switch
            value={enabled}
            onValueChange={(value) => toggle(server.name, value)}
            disabled={pending[server.name] !== undefined}
            accessibilityLabel={`${server.name} enabled`}
          />
        )}
      </View>;
    })}
    {data ? <Text style={[type.monoMeta, styles.footerMeta, { color: tokens.ink4 }]}>call timeout {data.call_timeout_secs}s · connect timeout {data.connect_timeout_secs}s · token values stay in your keyring — only the variable name is saved</Text> : null}
    <AddMcpServerSheet visible={adding} onClose={() => setAdding(false)} />
  </Screen>;
}

export default function McpScreen() {
  return <DesktopDrillDown><SettingsShell active="mcp"><McpScreenBody /></SettingsShell></DesktopDrillDown>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, headerRow: { flexDirection: "row", alignItems: "center" }, flexFill: { flex: 1 }, add: { fontSize: 15, fontWeight: "600" }, server: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12, minHeight: 56 }, dot: { width: 7, height: 7, borderRadius: 4 }, serverBody: { flex: 1, minWidth: 0, gap: 2 }, name: { fontSize: 14, fontFamily: monoFamily.bold }, footerMeta: { paddingTop: space.space4 } });
