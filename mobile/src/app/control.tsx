// Control tools: what an agent may do to this app on your behalf (start sessions, read a
// terminal, add schedules and MCP servers, check usage, notify you), one switch per tool, and
// the audit trail of what it did. Switched-off tools are neither offered to models nor runnable;
// everything that changes something still asks first in the session. Direct daemon connections
// only.
import { Wrench } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { Badge } from "../components/ds/Badge";
import { Banner } from "../components/ds/Banner";
import { EmptyState } from "../components/ds/EmptyState";
import { ListRow } from "../components/ds/ListRow";
import { PageHeader } from "../components/ds/PageHeader";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Skeleton } from "../components/ds/Skeleton";
import { Switch } from "../components/ds/Switch";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { auditTool, type ControlAuditEntry, type ControlTool } from "../lib/apiControl";
import { useControl, useControlAvailable, useSetControlTool } from "../lib/queriesControl";
import { SettingsShell } from "./(tabs)/settings";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { monoFamily, type as typeScale } from "../theme/typography";

function ToolRow({ tool, last, onToggle }: { tool: ControlTool; last: boolean; onToggle: (enabled: boolean) => void }) {
  return (
    <ListRow
      title={tool.name}
      subtitle={tool.description}
      leading={<Badge label={tool.kind === "read" ? "read" : "acts"} tone={tool.kind === "read" ? "neutral" : "warn"} lowercase />}
      trailing={<Switch value={tool.enabled} onValueChange={onToggle} accessibilityLabel={`${tool.name} ${tool.enabled ? "on" : "off"}`} />}
      hasInteractiveTrailing
      showSeparator={!last}
    />
  );
}

function AuditRow({ entry, last }: { entry: ControlAuditEntry; last: boolean }) {
  const tokens = useTokens();
  const tone = entry.decision === "allow" ? tokens.ink2 : tokens.danger;
  return (
    <View style={[styles.auditRow, last ? null : { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth }]}>
      <Text style={[typeScale.monoMeta, { color: tokens.ink }]} numberOfLines={1}>
        {auditTool(entry)} <Text style={{ color: tone }}>{entry.decision}</Text>
      </Text>
      <Text style={[typeScale.monoMeta, styles.detail, { color: tokens.ink3 }]} numberOfLines={2}>
        {entry.detail ?? ""}
      </Text>
      <Text style={[typeScale.meta, { color: tokens.ink4 }]}>{entry.ts.replace("T", " ").slice(0, 19)}</Text>
    </View>
  );
}

function ControlBody() {
  const tokens = useTokens();
  const toast = useToast();
  const direct = useControlAvailable();
  const query = useControl();
  const set = useSetControlTool();

  const toggle = (tool: ControlTool, enabled: boolean) => {
    set.mutate(
      { name: tool.name, enabled },
      { onError: (reason) => toast.show(reason instanceof Error ? reason.message : "Could not change the tool.", { tone: "danger" }) },
    );
  };

  const tools = query.data?.tools ?? [];
  const audit = query.data?.audit ?? [];

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <PageHeader title="Control tools" subtitle="What agents may do to this app on your behalf." />
      {!direct ? (
        <EmptyState icon={Wrench} message="Control tools are managed on the host. Connect to it directly to change them." />
      ) : query.isLoading ? (
        <View style={styles.skeletons}>
          {[0, 1, 2, 3, 4].map((i) => (
            <Skeleton key={i} width="80%" height={16} />
          ))}
        </View>
      ) : query.isError ? (
        <Banner tone="danger" message="Could not load the control tools." />
      ) : (
        <>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            Tools that act ask before they run, in every permission mode except full access. Switch one off and it disappears from models
            entirely. Also available to other agents as <Text style={{ fontFamily: monoFamily.regular, color: tokens.ink2 }}>forge mcp control</Text>.
          </Text>
          <View>
            {tools.map((tool, i) => (
              <ToolRow key={tool.name} tool={tool} last={i === tools.length - 1} onToggle={(enabled) => toggle(tool, enabled)} />
            ))}
          </View>
          <SectionHeader>Recent calls</SectionHeader>
          {audit.length === 0 ? (
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>No control tool has been called yet.</Text>
          ) : (
            <View>
              {audit.map((entry, i) => (
                <AuditRow key={`${entry.ts}-${i}`} entry={entry} last={i === audit.length - 1} />
              ))}
            </View>
          )}
        </>
      )}
    </Screen>
  );
}

export default function ControlScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="control">
        <ControlBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  skeletons: { gap: space.space12 },
  auditRow: { paddingVertical: space.space8, gap: 2 },
  detail: { flexShrink: 1 },
});
