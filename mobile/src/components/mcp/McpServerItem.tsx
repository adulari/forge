import { Pencil, Trash2 } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { McpServerRow } from "../../lib/api";
import { mcpStatusMeta, mcpToolSummary } from "../../lib/mcpManage";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { IconButton } from "../ds/IconButton";
import { Switch } from "../ds/Switch";

export interface McpServerItemProps {
  server: McpServerRow;
  enabled: boolean;
  checking: boolean;
  readOnlyReason: string | null;
  toggling: boolean;
  last: boolean;
  onToggle: (enabled: boolean) => void;
  onOpen: () => void;
  onEdit: () => void;
  onRemove: () => void;
}

export function McpServerItem({ server, enabled, checking, readOnlyReason, toggling, last, onToggle, onOpen, onEdit, onRemove }: McpServerItemProps) {
  const tokens = useTokens();
  const status = enabled ? (checking && (server.status == null || server.status === "unknown") ? undefined : server.status) : "disabled";
  const meta = mcpStatusMeta(status);
  const dot = meta.tone === "success" ? tokens.success : meta.tone === "danger" ? tokens.danger : meta.tone === "warn" ? tokens.warnBgInk : tokens.ink4;
  const statusText = checking && enabled && (server.status == null || server.status === "unknown") ? "Checking" : meta.label;
  const tools = enabled ? mcpToolSummary(server) : null;
  return (
    <View style={[styles.item, !last && { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth }]}>
      <View style={styles.top}>
        <Pressable
          onPress={onOpen}
          accessibilityRole="button"
          accessibilityLabel={`${server.name}, ${statusText}${tools ? `, ${tools}` : ""}. Show tools`}
          style={styles.main}
        >
          <View style={styles.nameRow}>
            <View style={[styles.dot, { backgroundColor: dot }]} />
            <Text style={[styles.name, { color: enabled ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>{server.name}</Text>
          </View>
          <View style={styles.badges}>
            <Badge label={server.transport} tone="outline" shape="small" lowercase />
            {server.scope ? <Badge label={server.scope} tone="outline" shape="small" lowercase /> : null}
            <Text style={[type.monoMeta, { color: meta.tone === "danger" ? tokens.danger : tokens.ink3 }]}>{[statusText, tools].filter(Boolean).join(" · ")}</Text>
          </View>
          {enabled && server.last_error ? <Text style={[type.monoMeta, { color: tokens.danger }]} numberOfLines={3} selectable>{server.last_error}</Text> : null}
          {readOnlyReason ? <Text style={[type.monoMeta, { color: tokens.warnBgInk }]}>{readOnlyReason}</Text> : null}
        </Pressable>
        {readOnlyReason ? null : (
          <View style={styles.controls}>
            <IconButton icon={<Pencil size={18} strokeWidth={1.75} color={tokens.ink2} />} onPress={onEdit} accessibilityLabel={`Edit ${server.name}`} />
            <IconButton icon={<Trash2 size={18} strokeWidth={1.75} color={tokens.danger} />} onPress={onRemove} accessibilityLabel={`Remove ${server.name}`} />
            <Switch value={enabled} onValueChange={onToggle} disabled={toggling} accessibilityLabel={`${server.name} enabled`} />
          </View>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  item: { paddingVertical: space.space8 },
  top: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  main: { flex: 1, minWidth: 0, gap: space.space4 },
  nameRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  name: { flexShrink: 1, fontSize: 14, fontFamily: monoFamily.bold },
  badges: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: space.space8 },
  controls: { flexDirection: "row", alignItems: "center", gap: space.space4 },
});
