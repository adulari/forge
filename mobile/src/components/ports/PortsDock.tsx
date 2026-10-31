import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { Copy, ExternalLink, MonitorPlay, Play, Plug, SquareTerminal, Square } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import type { ListeningPort, ProjectAction } from "../../lib/apiProjectActions";
import { useAuth } from "../../lib/auth";
import { setPreviewPreferences, previewLabel } from "../../lib/browserPreview";
import { isTauri } from "../../lib/platform";
import { useDirectBase, useProjectActions, useSessionPorts } from "../../lib/queriesProjectActions";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { EmptyState } from "../ds/EmptyState";
import { SectionHeader } from "../ds/SectionHeader";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { useOptionalWorkbench } from "../workbench/WorkbenchProvider";
import { createWorkbenchSurface } from "../workbench/model";
import { requestOpenActionOutput } from "./actionOutputBus";
import { rememberActionNames } from "./actionTerminalNames";
import { ActionGlyph } from "./actionIcons";
import {
  ACTIONS_TOML_SNIPPET,
  UNTRUSTED_HINT,
  actionStatus,
  exitBadge,
  hostBadge,
  localPortUrl,
  portLink,
  portsSummary,
  sourceLabel,
} from "./portsModel";
import { useActionControls } from "./useActionControls";

function RowButton({
  label,
  icon: Icon,
  onPress,
  accessibilityLabel,
  tone = "neutral",
  disabled = false,
}: {
  label: string;
  icon: React.ComponentType<{ size?: number; color?: string; strokeWidth?: number }>;
  onPress: () => void;
  accessibilityLabel: string;
  tone?: "neutral" | "accent" | "danger";
  disabled?: boolean;
}) {
  const tokens = useTokens();
  const [focused, setFocused] = useState(false);
  const ink = tone === "accent" ? tokens.accent : tone === "danger" ? tokens.danger : tokens.ink2;
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled }}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.button,
        {
          borderColor: focused ? tokens.accent : tone === "neutral" ? tokens.border : hexToRgba(ink, 0.45),
          backgroundColor: pressed || hovered ? tokens.bg3 : "transparent",
          opacity: disabled ? 0.45 : 1,
        },
      ]}
    >
      <Icon size={13} strokeWidth={1.75} color={ink} />
      <Text style={[typeScale.monoMeta, { color: ink }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

function ActionRow({
  action,
  pending,
  onRun,
  onStop,
  onOutput,
}: {
  action: ProjectAction;
  pending: boolean;
  onRun: () => void;
  onStop: () => void;
  onOutput: () => void;
}) {
  const tokens = useTokens();
  const status = actionStatus(action);
  const badge = exitBadge(action);
  return (
    <View style={[styles.row, { borderBottomColor: tokens.hairline }]} accessibilityLabel={`${action.name}, ${status}`}>
      <View style={styles.rowHead}>
        <ActionGlyph word={action.icon} size={15} color={status === "running" ? tokens.accent : tokens.ink3} />
        <Text style={[typeScale.bodyBold, styles.flex, { color: tokens.ink }]} numberOfLines={1}>
          {action.name}
        </Text>
        {status === "running" ? (
          <View style={styles.statusRow}>
            <StatusDot state="busy" accessibilityLabel={`${action.name} running`} />
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>running</Text>
          </View>
        ) : null}
        {badge ? <Badge label={badge} tone="danger" /> : null}
      </View>
      <Text style={[typeScale.monoMeta, styles.command, { color: tokens.ink3 }]} numberOfLines={2}>
        {action.cwd ? `${action.cwd}  ` : ""}$ {action.command}
      </Text>
      <View style={styles.buttons}>
        {status === "running" ? (
          <RowButton label="Stop" icon={Square} tone="danger" onPress={onStop} disabled={pending} accessibilityLabel={`Stop ${action.name}`} />
        ) : (
          <RowButton label="Run" icon={Play} tone="accent" onPress={onRun} disabled={pending} accessibilityLabel={`Run ${action.name}`} />
        )}
        {action.terminal_id ? (
          <RowButton
            label="Output"
            icon={SquareTerminal}
            accessibilityLabel={`Open output of ${action.name}`}
            onPress={onOutput}
          />
        ) : null}
      </View>
    </View>
  );
}

function PortRow({
  port,
  actions,
  sessionId,
  baseUrl,
}: {
  port: ListeningPort;
  actions: readonly ProjectAction[];
  sessionId: string;
  baseUrl: string | null;
}) {
  const tokens = useTokens();
  const toast = useToast();
  const workbench = useOptionalWorkbench();
  const link = portLink(port, baseUrl);
  const host = hostBadge(port.host);
  const label = `port ${port.port}`;

  const openPreview = () => {
    const surface = createWorkbenchSurface({ kind: "preview", sessionId, resourceId: `port-${port.port}`, title: `localhost:${port.port}` });
    setPreviewPreferences(previewLabel(surface.id), { url: localPortUrl(port.port) });
    workbench?.openSurface({ kind: "preview", sessionId, resourceId: `port-${port.port}`, title: `localhost:${port.port}` });
  };
  const open = () => {
    Linking.openURL(link.url).catch(() => toast.show("Could not open the link", { tone: "danger" }));
  };
  const copy = () => {
    void Clipboard.setStringAsync(link.url).then(() => toast.show(`Copied ${link.url}`, { tone: "neutral" }));
  };

  return (
    <View style={[styles.row, { borderBottomColor: tokens.hairline }]}>
      <View style={styles.rowHead}>
        <Text style={[typeScale.bodyBold, { color: tokens.ink, fontFamily: monoFamily.regular }]} accessibilityLabel={label}>
          :{port.port}
        </Text>
        <Text style={[typeScale.monoMeta, styles.flex, { color: tokens.ink3 }]} numberOfLines={1}>
          {port.process}
        </Text>
        <Badge label={sourceLabel(port, actions)} tone={port.source === "action" ? "accent" : "neutral"} lowercase />
        <Badge label={host.label} tone="outline" lowercase />
      </View>
      {port.command ? (
        <Text style={[typeScale.monoMeta, styles.command, { color: tokens.ink4 }]} numberOfLines={1}>
          {port.command}
        </Text>
      ) : null}
      <View style={styles.buttons}>
        {isTauri ? (
          <RowButton label="Preview" icon={MonitorPlay} tone="accent" onPress={openPreview} accessibilityLabel={`Open ${label} in the browser preview`} />
        ) : null}
        <RowButton label="Open" icon={ExternalLink} onPress={open} accessibilityLabel={`Open ${link.url} in a new tab`} />
        <RowButton label="Copy URL" icon={Copy} onPress={copy} accessibilityLabel={`Copy ${link.url}`} />
      </View>
      {link.note ? <Text style={[typeScale.monoMeta, { color: tokens.warn }]}>{link.note}</Text> : null}
    </View>
  );
}

function SnippetBlock() {
  const tokens = useTokens();
  const toast = useToast();
  return (
    <View style={[styles.snippet, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <Text selectable style={[typeScale.monoMeta, { color: tokens.ink2 }]}>
        {ACTIONS_TOML_SNIPPET}
      </Text>
      <RowButton
        label="Copy snippet"
        icon={Copy}
        accessibilityLabel="Copy the actions snippet"
        onPress={() => void Clipboard.setStringAsync(ACTIONS_TOML_SNIPPET).then(() => toast.show("Snippet copied", { tone: "neutral" }))}
      />
    </View>
  );
}

export function PortsDock({ sessionId, onNavigate }: { sessionId: string | null; onNavigate?: () => void }) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const direct = useDirectBase();
  const actions = useProjectActions(sessionId);
  const ports = useSessionPorts(sessionId);
  const controls = useActionControls();
  const actionRows = actions.data?.actions ?? [];

  useEffect(() => {
    if (actions.data) rememberActionNames(actions.data.actions);
  }, [actions.data]);

  if (!sessionId) return <EmptyState icon={Plug} message="Open a session to see its actions and ports." />;
  if (!direct) return <EmptyState icon={Plug} message="Actions and ports need a direct connection to the daemon, not the relay." />;

  const portRows = ports.data ?? [];

  return (
    <ScrollView contentContainerStyle={styles.content} accessibilityLabel="Ports and project actions">
      <SectionHeader>Actions</SectionHeader>
      {actions.isPending ? (
        <ActivityIndicator color={tokens.accent} accessibilityLabel="Loading actions" style={styles.loader} />
      ) : actions.isError ? (
        <Text style={[typeScale.sub, { color: tokens.danger }]}>Could not load actions: {actions.error instanceof Error ? actions.error.message : "unknown error"}</Text>
      ) : actionRows.length > 0 ? (
        actionRows.map((action) => (
          <ActionRow
            key={action.id}
            action={action}
            pending={controls.pendingActionId === action.id}
            onRun={() => controls.run(sessionId, action)}
            onStop={() => controls.stop(sessionId, action)}
            onOutput={() => {
              const shown = requestOpenActionOutput({ sessionId, terminalId: action.terminal_id as string, title: action.name });
              if (!shown) {
                onNavigate?.();
                router.push({ pathname: "/session/[id]/terminal", params: { id: sessionId } });
              }
            }}
          />
        ))
      ) : actions.data?.untrusted ? (
        <Text style={[typeScale.sub, { color: tokens.warn }]} accessibilityRole="alert">
          {UNTRUSTED_HINT}
        </Text>
      ) : (
        <View style={styles.empty}>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            Add [[actions]] to .forge/config.toml to run dev scripts from here.
          </Text>
          <SnippetBlock />
        </View>
      )}

      <SectionHeader>Listening ports</SectionHeader>
      {ports.isPending ? (
        <ActivityIndicator color={tokens.accent} accessibilityLabel="Loading ports" style={styles.loader} />
      ) : ports.isError ? (
        <Text style={[typeScale.sub, { color: tokens.danger }]}>Could not scan ports: {ports.error instanceof Error ? ports.error.message : "unknown error"}</Text>
      ) : portRows.length === 0 ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          {portsSummary(0)}. A dev server started by an action, a terminal or an agent in this workspace shows up here.
        </Text>
      ) : (
        portRows.map((port) => (
          <PortRow key={`${port.port}-${port.pid}`} port={port} actions={actionRows} sessionId={sessionId} baseUrl={baseUrl} />
        ))
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space12, gap: space.space8 },
  flex: { flex: 1 },
  loader: { alignSelf: "flex-start", margin: space.space8 },
  row: { gap: space.space4, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  rowHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  statusRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  command: { paddingLeft: 23 },
  buttons: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, paddingTop: space.space4 },
  button: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    minHeight: 28,
    paddingHorizontal: space.space8,
    borderWidth: 1,
    borderRadius: radii.radius4,
  },
  empty: { gap: space.space8 },
  snippet: { gap: space.space8, padding: space.space12, borderWidth: 1, borderRadius: radii.radius4, alignItems: "flex-start" },
});
