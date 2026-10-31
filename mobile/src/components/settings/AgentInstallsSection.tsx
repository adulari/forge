// Subscription CLI install state on Providers: installed vs latest, per-agent progress while the
// vendor's installer or updater runs on the host, and one "update all" for everything outdated.
import { Download, RefreshCw } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import {
  actionLabel,
  anyJobActive,
  confirmMessage,
  outdatedAgents,
  isJobActive,
  jobOutcome,
  outdatedSummary,
  runtimeLabel,
  useAgentInstalls,
  useRefreshAgentInstalls,
  useRunAgentInstall,
  useUpdateAllAgents,
  versionLine,
  type AgentInstall,
} from "../../lib/agentInstalls";
import { ApiError } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { SectionHeader } from "../ds/SectionHeader";
import { useToast } from "../ds/ToastHost";

function failure(error: unknown, fallback: string): string {
  return error instanceof ApiError || error instanceof Error ? error.message : fallback;
}

function AgentRow({
  agent,
  canRun,
  busy,
  onRun,
}: {
  agent: AgentInstall;
  canRun: boolean;
  busy: boolean;
  onRun: (agent: AgentInstall) => void;
}) {
  const tokens = useTokens();
  const active = isJobActive(agent.job);
  const label = actionLabel(agent);
  const tail = agent.job && (active || agent.job.state === "failed") ? agent.job.lines.slice(-4) : [];
  return (
    <View style={[styles.row, { borderBottomColor: tokens.hairline }]}>
      <View style={styles.head}>
        <View style={styles.title}>
          <Text style={[type.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
            {agent.name}
          </Text>
          <Text
            style={[type.monoMeta, { color: agent.outdated ? tokens.warnBgInk : tokens.ink4 }]}
            numberOfLines={2}
          >
            {versionLine(agent)}
          </Text>
        </View>
        {agent.installed && agent.runtime !== "native" ? (
          <Badge label={runtimeLabel(agent.runtime)} tone="outline" />
        ) : null}
        {agent.outdated ? <Badge label="outdated" tone="warn" /> : null}
        {active ? (
          <ActivityIndicator color={tokens.accent} accessibilityLabel={`${agent.name} ${agent.job?.state}`} />
        ) : label ? (
          <Button
            label={label}
            variant={agent.outdated || !agent.installed ? "secondary" : "ghost"}
            disabled={!canRun || busy}
            onPress={() => onRun(agent)}
            accessibilityLabel={`${label} ${agent.name}`}
          />
        ) : null}
      </View>
      {!agent.installed && agent.manual_hint ? (
        <Text style={[type.sub, { color: tokens.ink3 }]}>{agent.manual_hint}</Text>
      ) : null}
      {label && agent.command && !active ? (
        <Text style={[type.monoMeta, { color: tokens.ink4 }]}>runs: {agent.command}</Text>
      ) : null}
      {tail.length > 0 ? (
        <View style={[styles.log, { backgroundColor: tokens.bg2 }]} accessibilityLiveRegion="polite">
          {tail.map((line, index) => (
            <Text key={`${index}-${line}`} style={[type.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
              {line}
            </Text>
          ))}
        </View>
      ) : null}
      {agent.job?.state === "failed" && agent.job.error ? (
        <Text style={[type.sub, { color: tokens.danger }]}>{agent.job.error}</Text>
      ) : null}
    </View>
  );
}

export function AgentInstallsSection({ onChanged }: { onChanged?: () => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const query = useAgentInstalls();
  const run = useRunAgentInstall();
  const updateAll = useUpdateAllAgents();
  const refresh = useRefreshAgentInstalls();
  const [pending, setPending] = useState<AgentInstall[] | null>(null);
  const wasActive = useRef(new Map<string, boolean>());
  const agents = query.data?.agents ?? [];
  const active = anyJobActive(agents);

  // A job that was running on the last poll and is not now has just finished: say how it ended
  // once, and let the provider rows pick up the new version.
  useEffect(() => {
    let finished = false;
    for (const agent of agents) {
      const running = isJobActive(agent.job);
      if (wasActive.current.get(agent.id) && !running) {
        const message = jobOutcome(agent);
        if (message) toast.show(message, { tone: agent.job?.state === "failed" ? "danger" : "success" });
        finished = true;
      }
      wasActive.current.set(agent.id, running);
    }
    if (finished) onChanged?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query.data]);

  if (!query.data || agents.length === 0) return null;
  const summary = outdatedSummary(query.data.outdated);
  const start = async (agent: AgentInstall) => {
    try {
      await run.mutateAsync(agent.id);
    } catch (error) {
      toast.show(failure(error, `Could not start ${agent.name}.`), { tone: "danger" });
    }
  };
  const startAll = async () => {
    try {
      await updateAll.mutateAsync();
    } catch (error) {
      toast.show(failure(error, "Could not start the updates."), { tone: "danger" });
    }
  };
  const recheck = async () => {
    try {
      await refresh();
    } catch (error) {
      toast.show(failure(error, "Could not check for new versions."), { tone: "danger" });
    }
  };

  return (
    <View>
      <SectionHeader>Agent CLIs</SectionHeader>
      <View style={styles.summary}>
        <Text style={[type.sub, styles.summaryText, { color: summary ? tokens.warnBgInk : tokens.ink3 }]}>
          {summary ?? "Subscription CLIs on this host. Forge runs each vendor's own installer or updater."}
        </Text>
        {query.data.outdated > 0 ? (
          <Button
            label="Update all"
            variant="secondary"
            icon={<Download size={15} strokeWidth={1.75} color={tokens.ink2} />}
            disabled={!query.data.can_run || active || updateAll.isPending}
            onPress={() => setPending(outdatedAgents(agents))}
            accessibilityLabel="Update all outdated agents"
          />
        ) : null}
        <Button
          label="Check"
          variant="ghost"
          icon={<RefreshCw size={15} strokeWidth={1.75} color={tokens.ink3} />}
          disabled={active}
          onPress={() => void recheck()}
          accessibilityLabel="Check for new agent versions"
        />
      </View>
      {!query.data.can_run ? (
        <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
          Installs run only from the machine hosting the daemon. Open Forge there to update these.
        </Text>
      ) : null}
      {agents.map((agent) => (
        <AgentRow
          key={agent.id}
          agent={agent}
          canRun={query.data.can_run}
          busy={run.isPending || updateAll.isPending}
          onRun={(row) => setPending([row])}
        />
      ))}
      <ConfirmDialog
        visible={pending != null}
        title={pending && pending.length > 1 ? "Update all agent CLIs?" : `${pending?.[0] ? actionLabel(pending[0]) : "Run"} ${pending?.[0]?.name ?? ""}?`}
        message={pending ? confirmMessage(pending) : ""}
        confirmLabel="Run"
        onConfirm={() => {
          const chosen = pending;
          setPending(null);
          if (!chosen) return;
          if (chosen.length > 1) void startAll();
          else if (chosen[0]) void start(chosen[0]);
        }}
        onCancel={() => setPending(null)}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  summary: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space4 },
  summaryText: { flex: 1, minWidth: 0 },
  note: { paddingBottom: space.space4 },
  row: { borderBottomWidth: StyleSheet.hairlineWidth, paddingVertical: space.space12, gap: space.space8 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8, flexWrap: "wrap" },
  title: { flex: 1, minWidth: 140, gap: 2 },
  log: { borderRadius: radii.radius4, paddingHorizontal: space.space8, paddingVertical: space.space4, gap: 2 },
});
