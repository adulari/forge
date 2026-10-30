// Machined MCP servers ("D Settings Plans MCP Config" L869-891 / "M Config MCP Plans" L384-401):
// dense mono rows with a live enable/disable Switch driven by `PATCH /api/mcp`.
//
// The daemon only owns the two `mcp.toml` files the CLI writes, so a server that resolves from an
// imported `.mcp.json` is refused with a 404 rather than being forked into a new `mcp.toml` entry.
// `McpServerRow` carries no "which file defines this" field, so that case is undetectable until
// the first PATCH comes back — the row is then latched read-only for the rest of the session with
// the daemon's own explanation, instead of leaving a toggle that can only ever fail.
import React, { useCallback, useEffect, useRef, useState } from "react";
import { RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { AddMcpServerSheet } from "../components/mcp/AddMcpServerSheet";
import { Badge } from "../components/ds/Badge";
import { PageHeader } from "../components/ds/PageHeader";
import { Button } from "../components/ds/Button";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { Skeleton } from "../components/ds/Skeleton";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { ApiError } from "../lib/api";
import { useMcp, useUpdateMcpServer } from "../lib/queries";
import { useDeleteMcpServer, useReloadMcp, useUpdateMcpBuiltin } from "../lib/mcpManage";
import type { McpServerRow } from "../lib/api";
import { McpProbeSheet } from "../components/mcp/McpProbeSheet";
import { McpServerItem } from "../components/mcp/McpServerItem";
import { ConfirmDialog } from "../components/ds/ConfirmDialog";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Plug, Plus, RefreshCw } from "lucide-react-native";
import { IconButton } from "../components/ds/IconButton";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";
import { usePullRefresh } from "../lib/usePullRefresh";
import { useAuth } from "../lib/auth";
import { supportsDirectDaemonEndpoints } from "../lib/transport";

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
  const pull = usePullRefresh(query.refetch);
  const update = useUpdateMcpServer();
  const reload = useReloadMcp();
  const remove = useDeleteMcpServer();
  const builtin = useUpdateMcpBuiltin();
  const data = query.data;
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<McpServerRow | null>(null);
  const [probing, setProbing] = useState<McpServerRow | null>(null);
  const [removing, setRemoving] = useState<McpServerRow | null>(null);
  // The relay refuses MCP writes by design: registering or enabling a server persists a command
  // the host later runs. Over Anywhere the screen reviews only, instead of offering an Add sheet
  // and toggles whose requests always come back "not allowlisted".
  const { baseUrl } = useAuth();
  const direct = baseUrl == null || supportsDirectDaemonEndpoints(baseUrl);
  // Desired-state overlay keyed by server name. The mutation seeds the cache with the daemon's
  // full refreshed list, so the overlay is dropped on settle — success keeps the new value,
  // failure rolls back to whatever the server still reports.
  const [pending, setPending] = useState<Record<string, boolean>>({});
  const [external, setExternal] = useState<Record<string, string>>({});

  const { mutate: runReload } = reload;
  const autoChecked = useRef(false);
  useEffect(() => {
    if (!direct || autoChecked.current || !data) return;
    autoChecked.current = true;
    if (data.servers.some((server) => server.enabled && server.status === "unknown")) runReload();
  }, [data, direct, runReload]);

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
          onSuccess: () => {
            if (enabled) runReload();
          },
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
    [toast, update, runReload],
  );

  const confirmRemove = () => {
    if (!removing) return;
    const name = removing.name;
    remove.mutate(name, {
      onSuccess: () => setRemoving(null),
      onError: (error) => {
        setRemoving(null);
        toast.show(error instanceof ApiError ? (error.body as { error?: string } | undefined)?.error ?? error.message : `could not remove ${name}`, { tone: "danger" });
      },
    });
  };

  return <Screen scroll refreshControl={<RefreshControl refreshing={pull.refreshing} onRefresh={pull.onRefresh} />} contentContainerStyle={styles.content}>
    <PageHeader
      title="MCP servers"
      subtitle="External tools available to Forge. Secrets remain on the host."
      trailing={direct ? <>
        <IconButton icon={<RefreshCw size={18} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => runReload()} disabled={reload.isPending} accessibilityLabel={reload.isPending ? "Checking MCP servers" : "Reload MCP servers"} />
        <Button label="Add" variant="secondary" icon={<Plus size={14} color={tokens.ink2} />} onPress={() => setAdding(true)} accessibilityLabel="Add an MCP server" />
      </> : null}
    />
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
    {data?.builtins && data.builtins.length > 0 ? (
      <View>
        <SectionHeader>Built-in</SectionHeader>
        {data.builtins.map((item, index) => (
          <View key={item.id} style={[styles.builtin, index < data.builtins!.length - 1 ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
            <View style={styles.serverBody}>
              <Text style={[styles.name, { color: tokens.ink }]} numberOfLines={1}>{item.label}</Text>
              <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={2}>{[item.detail, item.tool_count != null ? `${item.tool_count} ${item.tool_count === 1 ? "tool" : "tools"}` : null].filter(Boolean).join(" · ")}</Text>
            </View>
            {item.toggleable && direct ? (
              <Switch
                value={item.enabled}
                onValueChange={(value) => builtin.mutate({ id: item.id, enabled: value }, { onError: () => toast.show(`could not update ${item.label}`, { tone: "danger" }) })}
                disabled={builtin.isPending}
                accessibilityLabel={`${item.label} enabled`}
              />
            ) : (
              <Badge label={item.enabled ? "always on" : "off"} tone={item.enabled ? "success" : "neutral"} />
            )}
          </View>
        ))}
      </View>
    ) : null}
    {data ? <SectionHeader>Servers</SectionHeader> : null}
    {!query.isLoading && data?.servers.length === 0 ? <EmptyState icon={Plug} message="No MCP servers configured." /> : null}
    {data?.servers.map((server, index) => {
      const enabled = pending[server.name] ?? server.enabled;
      // The daemon now says up front whether it can write this server (v9); the latched 404
      // reason stays as the fallback for a pre-v9 daemon that omits the flag.
      const readOnlyReason =
        external[server.name] ??
        (server.editable === false ? "defined outside mcp.toml — edit it where it is defined" : null) ??
        (direct ? null : "change it on the host — Forge Anywhere does not carry MCP changes");
      return <McpServerItem
        key={server.name}
        server={server}
        enabled={enabled}
        checking={reload.isPending}
        readOnlyReason={readOnlyReason}
        toggling={pending[server.name] !== undefined}
        last={index === data.servers.length - 1}
        onToggle={(value) => toggle(server.name, value)}
        onOpen={() => setProbing(server)}
        onEdit={() => setEditing(server)}
        onRemove={() => setRemoving(server)}
      />;
    })}
    {data ? <Text style={[type.monoMeta, styles.footerMeta, { color: tokens.ink4 }]}>call timeout {data.call_timeout_secs}s · connect timeout {data.connect_timeout_secs}s · token values stay in your keyring — only the variable name is saved</Text> : null}
    <AddMcpServerSheet visible={adding || editing != null} editing={editing} onClose={() => { setAdding(false); setEditing(null); }} />
    <McpProbeSheet server={probing} canEdit={direct && probing?.editable !== false} onClose={() => setProbing(null)} />
    <ConfirmDialog
      visible={removing != null}
      title={`Remove ${removing?.name ?? "server"}?`}
      message="Deletes its entry from mcp.toml. Stored tokens in your keyring are not touched."
      confirmLabel="Remove"
      destructive
      loading={remove.isPending}
      onConfirm={confirmRemove}
      onCancel={() => setRemoving(null)}
    />
  </Screen>;
}

export default function McpScreen() {
  return <DesktopDrillDown><SettingsShell active="mcp"><McpScreenBody /></SettingsShell></DesktopDrillDown>;
}

const styles = StyleSheet.create({ content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 }, headerRow: { flexDirection: "row", alignItems: "center" }, flexFill: { flex: 1 }, add: { fontSize: 15, fontWeight: "600" }, addButton: { minHeight: 44, justifyContent: "center", paddingHorizontal: space.space8 }, builtin: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12, minHeight: 56 }, server: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space12, minHeight: 56 }, dot: { width: 7, height: 7, borderRadius: 4 }, serverBody: { flex: 1, minWidth: 0, gap: 2 }, name: { fontSize: 14, fontFamily: monoFamily.bold }, footerMeta: { paddingTop: space.space4 } });
