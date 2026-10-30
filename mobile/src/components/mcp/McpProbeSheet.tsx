import React, { useEffect, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import type { McpServerRow } from "../../lib/api";
import { mcpStatusMeta, toggleToolDisabled, useProbeMcp, useSetDisabledTools } from "../../lib/mcpManage";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";

/** Connects to one server on demand and lists what it exposes; each tool can be switched off for the model. */
export function McpProbeSheet({ server, onClose, canEdit }: { server: McpServerRow | null; onClose: () => void; canEdit: boolean }) {
  const tokens = useTokens();
  const toast = useToast();
  const probe = useProbeMcp();
  const setDisabled = useSetDisabledTools();
  const [disabled, setLocalDisabled] = useState<string[]>([]);
  const name = server?.name;
  const { mutate } = probe;
  useEffect(() => {
    if (name) mutate(name);
  }, [name, mutate]);
  useEffect(() => {
    if (probe.data) setLocalDisabled(probe.data.disabled_tools);
  }, [probe.data]);

  const toggle = (tool: string, enabled: boolean) => {
    if (!name) return;
    const previous = disabled;
    const next = toggleToolDisabled(disabled, tool, enabled);
    setLocalDisabled(next);
    setDisabled.mutate(
      { name, disabled_tools: next },
      {
        onError: () => {
          setLocalDisabled(previous);
          toast.show(`could not update ${tool}`, { tone: "danger" });
        },
      },
    );
  };

  const meta = mcpStatusMeta(probe.data?.status);
  const tools = probe.data?.tools ?? [];
  return (
    <Sheet visible={server != null} onClose={onClose} accessibilityLabel={`${name ?? "MCP server"} tools`} snapPoints={[0.85]}>
      <ScrollView contentContainerStyle={styles.content}>
        <View style={styles.titleRow}>
          <Text style={[type.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>{name}</Text>
          {probe.data ? <Badge label={meta.label} tone={meta.tone} /> : null}
        </View>
        {probe.isPending ? (
          <View style={styles.gap}>
            <Skeleton width="50%" height={14} />
            <Skeleton width="80%" height={12} />
            <Skeleton width="70%" height={12} />
          </View>
        ) : null}
        {probe.isError ? <Text style={[type.sub, { color: tokens.danger }]}>{probe.error instanceof Error ? probe.error.message : "Could not probe this server."}</Text> : null}
        {probe.data?.last_error ? <Text style={[type.monoMeta, { color: tokens.danger }]} selectable>{probe.data.last_error}</Text> : null}
        {probe.data && tools.length === 0 && !probe.data.last_error ? <Text style={[type.sub, { color: tokens.ink3 }]}>This server exposes no tools.</Text> : null}
        {tools.length > 0 ? <SectionHeader>{`Tools · ${tools.length - disabled.filter((d) => tools.some((t) => t.name === d)).length} of ${tools.length} on`}</SectionHeader> : null}
        {tools.map((tool) => {
          const on = !disabled.includes(tool.name);
          return (
            <View key={tool.name} style={[styles.tool, { borderBottomColor: tokens.hairline }]}>
              <View style={styles.toolBody}>
                <Text style={[styles.toolName, { color: on ? tokens.ink : tokens.ink3 }]} numberOfLines={1}>{tool.name}</Text>
                {tool.description ? <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={2}>{tool.description}</Text> : null}
              </View>
              {canEdit ? <Switch value={on} onValueChange={(value) => toggle(tool.name, value)} accessibilityLabel={`${tool.name} visible to the model`} /> : <Badge label={on ? "on" : "off"} tone={on ? "success" : "neutral"} />}
            </View>
          );
        })}
        {probe.data && probe.data.resources.length > 0 ? <SectionHeader>{`Resources · ${probe.data.resources.length}`}</SectionHeader> : null}
        {probe.data?.resources.map((resource) => (
          <View key={resource.uri} style={styles.toolBody}>
            <Text style={[styles.toolName, { color: tokens.ink }]} numberOfLines={1}>{resource.name}</Text>
            <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>{resource.uri}</Text>
          </View>
        ))}
        <Button label="Probe again" variant="secondary" onPress={() => name && mutate(name)} disabled={probe.isPending} fullWidth />
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1, minWidth: 0 },
  gap: { gap: space.space8 },
  tool: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space8, minHeight: 48, borderBottomWidth: StyleSheet.hairlineWidth },
  toolBody: { flex: 1, minWidth: 0, gap: 2 },
  toolName: { fontSize: 13, fontFamily: monoFamily.bold },
});
