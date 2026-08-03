import * as Clipboard from "expo-clipboard";
import * as Updates from "expo-updates";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { ActivityIndicator, StyleSheet, Text, View } from "react-native";

import { BackLink } from "../components/ds/BackLink";
import { Badge, type BadgeTone } from "../components/ds/Badge";
import { Banner } from "../components/ds/Banner";
import { Button } from "../components/ds/Button";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { useToast } from "../components/ds/ToastHost";
import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { formatBytes } from "../lib/anywhere/format";
import { ApiError } from "../lib/api";
import { useAppVersion } from "../lib/appVersion";
import { assessCompatibility, buildSupportSummary } from "../lib/diagnostics";
import { getDesktopPerformanceSnapshot, type DesktopPerformanceSnapshot } from "../lib/performance";
import { isTauri } from "../lib/platform";
import { useDiagnostics } from "../lib/queries";
import { PROTOCOL_VERSION } from "../lib/remoteProtocol";
import {
  checkDesktopUpdate,
  getDesktopUpdateState,
  installDesktopUpdate,
  useDesktopUpdateState,
} from "../lib/updater";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { SettingsShell } from "./(tabs)/settings";

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  const days = Math.floor(hours / 24);
  return `${days}d ${hours % 24}h`;
}

function compatibilityTone(status: ReturnType<typeof assessCompatibility>["status"]): BadgeTone {
  if (status === "compatible") return "success";
  if (status === "version-skew") return "neutral";
  if (status === "unknown") return "outline";
  return "warn";
}

function updateLabel(phase: ReturnType<typeof useDesktopUpdateState>["phase"]): string {
  switch (phase) {
    case "checking": return "Checking…";
    case "up-to-date": return "Up to date";
    case "available": return "Available";
    case "installing": return "Installing…";
    case "error": return "Needs attention";
    case "idle":
    default: return "Not checked";
  }
}

export default function DiagnosticsScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const query = useDiagnostics();
  const appVersion = useAppVersion();
  const update = useDesktopUpdateState();
  const nativeRuntimeVersion = Updates.runtimeVersion ?? null;
  const [performanceSnapshot, setPerformanceSnapshot] = useState<DesktopPerformanceSnapshot>(() => getDesktopPerformanceSnapshot());
  const diagnostics = query.data;
  const compatibility = useMemo(
    () => assessCompatibility(
      diagnostics?.host.protocol,
      diagnostics?.host.version,
      PROTOCOL_VERSION,
      appVersion,
    ),
    [appVersion, diagnostics?.host.protocol, diagnostics?.host.version],
  );

  useEffect(() => {
    const timer = setInterval(() => setPerformanceSnapshot(getDesktopPerformanceSnapshot()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (isTauri && update.phase === "idle") {
      void checkDesktopUpdate().catch(() => undefined);
    }
  }, [update.phase]);

  const refresh = useCallback(async () => {
    const result = await query.refetch();
    if (isTauri) await checkDesktopUpdate().catch(() => undefined);
    if (result.isError) {
      toast.show("couldn't refresh daemon diagnostics.", { tone: "danger" });
    } else {
      toast.show("Diagnostics refreshed.", { tone: "neutral" });
    }
  }, [query, toast]);

  const install = useCallback(() => {
    void installDesktopUpdate().catch(() => {
      toast.show("couldn't install the desktop update.", { tone: "danger" });
    });
  }, [toast]);

  const checkUpdate = useCallback(() => {
    void checkDesktopUpdate()
      .then(() => {
        if (getDesktopUpdateState().phase !== "available") {
          toast.show("Desktop update check finished.", { tone: "neutral" });
        }
      })
      .catch(() => toast.show("couldn't check for desktop updates.", { tone: "danger" }));
  }, [toast]);

  const copySummary = useCallback(() => {
    if (!diagnostics) return;
    const summary = buildSupportSummary(
      diagnostics,
      appVersion,
      PROTOCOL_VERSION,
      update,
      nativeRuntimeVersion,
    );
    void Clipboard.setStringAsync(summary)
      .then(() => toast.show("Copied sanitized support summary.", { tone: "neutral" }))
      .catch(() => toast.show("couldn't copy the support summary.", { tone: "danger" }));
  }, [appVersion, diagnostics, nativeRuntimeVersion, toast, update]);

  const oldDaemon = query.error instanceof ApiError && query.error.status === 404;

  return (
    <DesktopDrillDown>
      <SettingsShell active="diagnostics">
        <Screen scroll contentContainerStyle={styles.content}>
          <View style={styles.header}>
            <BackLink />
            <Text accessibilityRole="header" style={[type.title, { color: tokens.ink }]}>
              Diagnostics & updates
            </Text>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              A bounded operational view. Forge never includes tokens, credentials, paths, prompts,
              environment values, or log contents here.
            </Text>
          </View>

          {query.isLoading ? <ActivityIndicator color={tokens.ink3} /> : null}
          {oldDaemon ? (
            <Banner
              tone="warn"
              message="This daemon is too old to report diagnostics. Run `forge update`, restart `forge serve`, then refresh."
              actionLabel="Refresh"
              onAction={() => void refresh()}
            />
          ) : query.isError ? (
            <Banner
              tone="danger"
              message="Forge couldn't load daemon diagnostics. The active server may be offline."
              actionLabel="Retry"
              onAction={() => void refresh()}
            />
          ) : null}

          {diagnostics ? (
            <>
              <View>
                <SectionHeader>Compatibility</SectionHeader>
                {compatibility.status === "daemon-outdated" ? (
                  <Banner
                    tone="warn"
                    message={`${compatibility.detail} Run \`forge update\` on the host, then restart \`forge serve\`.`}
                  />
                ) : compatibility.status === "client-outdated" ? (
                  <Banner
                    tone="warn"
                    message={`${compatibility.detail} Update this Forge app before continuing.`}
                  />
                ) : null}
                <ListRow
                  title={compatibility.title}
                  subtitle={compatibility.detail}
                  trailing={<Badge label={compatibility.status.replace("-", " ")} tone={compatibilityTone(compatibility.status)} />}
                />
                <ListRow
                  title={`App v${appVersion}`}
                  subtitle={`Remote protocol v${PROTOCOL_VERSION}`}
                />
                {nativeRuntimeVersion ? (
                  <ListRow
                    title={nativeRuntimeVersion}
                    subtitle="Embedded native/OTA runtime fingerprint"
                  />
                ) : null}
                <ListRow
                  title={`Daemon v${diagnostics.host.version}`}
                  subtitle={`Remote protocol v${diagnostics.host.protocol}`}
                  showSeparator={false}
                />
              </View>

              <View>
                <SectionHeader>Host</SectionHeader>
                <ListRow title={diagnostics.host.hostname} subtitle="Daemon host name" />
                <ListRow
                  title={`${diagnostics.host.os} · ${diagnostics.host.arch}`}
                  subtitle={`PID ${diagnostics.host.pid} · uptime ${formatUptime(diagnostics.host.process_uptime_secs)}`}
                />
                <ListRow
                  title={new Date(diagnostics.checked_at * 1000).toLocaleString()}
                  subtitle="Daemon snapshot checked"
                  showSeparator={false}
                />
              </View>

              <View>
                <SectionHeader>Runtime</SectionHeader>
                <ListRow
                  title={`${diagnostics.runtime.sessions} sessions`}
                  subtitle={`${diagnostics.runtime.busy_sessions} busy · ${diagnostics.runtime.waiting_sessions} waiting`}
                />
                <ListRow
                  title={`${diagnostics.runtime.terminals} terminals`}
                  subtitle={`${diagnostics.runtime.terminal_clients} attached clients`}
                />
                <ListRow
                  title="Notification senders"
                  subtitle={`Web ${diagnostics.runtime.web_push_ready ? "ready" : "not configured"} · native ${diagnostics.runtime.native_push_ready ? "ready" : "not configured"}`}
                  showSeparator={false}
                />
              </View>

              <View>
                <SectionHeader>Resources</SectionHeader>
                <ListRow
                  title={`Forge process ${formatBytes(diagnostics.resources.process_memory_bytes)}`}
                  subtitle={`${formatBytes(diagnostics.resources.process_virtual_memory_bytes)} virtual memory`}
                />
                <ListRow
                  title={`${formatBytes(diagnostics.resources.system_available_memory_bytes)} available`}
                  subtitle={`${formatBytes(diagnostics.resources.system_total_memory_bytes)} system memory`}
                />
                <ListRow
                  title={`${diagnostics.resources.cpu_count} logical CPUs`}
                  subtitle={`Load ${diagnostics.resources.load_average_one.toFixed(2)} · ${diagnostics.resources.load_average_five.toFixed(2)} · ${diagnostics.resources.load_average_fifteen.toFixed(2)}`}
                  showSeparator={false}
                />
              </View>

              <View>
                <SectionHeader>Host checks</SectionHeader>
                {diagnostics.checks.map((check, index) => (
                  <View key={check.id}>
                    <ListRow
                      title={check.label}
                      subtitle={check.detail}
                      trailing={<Badge label={check.status} tone={check.status === "ok" ? "success" : "warn"} />}
                      showSeparator={check.fix != null || index < diagnostics.checks.length - 1}
                    />
                    {check.fix ? (
                      <Text style={[type.sub, styles.fix, { color: tokens.warnBgInk }]}>
                        Fix: {check.fix}
                      </Text>
                    ) : null}
                  </View>
                ))}
              </View>

              <View>
                <SectionHeader>Desktop performance</SectionHeader>
                <ListRow
                  title={performanceSnapshot.startupToInteractiveMs == null ? "Startup pending" : `Interactive in ${performanceSnapshot.startupToInteractiveMs.toFixed(0)} ms`}
                  subtitle="Measured from module load until the paired app became interactive"
                />
                <ListRow
                  title={`${performanceSnapshot.composerInputToPaintP50Ms == null ? "No composer samples" : `Composer p50 ${performanceSnapshot.composerInputToPaintP50Ms.toFixed(1)} ms`}`}
                  subtitle={`${performanceSnapshot.composerInputSamples} input samples · max ${performanceSnapshot.composerInputToPaintMaxMs?.toFixed(1) ?? "—"} ms`}
                />
                <ListRow
                  title={`${performanceSnapshot.frameSamples} frame intervals · ${performanceSnapshot.droppedFrames} estimated dropped`}
                  subtitle={`Estimated refresh ${performanceSnapshot.estimatedRefreshRateHz?.toFixed(1) ?? "—"} Hz · long tasks ${performanceSnapshot.longTaskCount} (${performanceSnapshot.longestTaskMs.toFixed(1)} ms max)`}
                />
                <ListRow
                  title={`Long-task time ${performanceSnapshot.longTaskTotalMs.toFixed(1)} ms`}
                  subtitle="Browser/WebView PerformanceObserver; unavailable entries are excluded"
                  showSeparator={false}
                />
              </View>

              {isTauri ? (
                <View>
                  <SectionHeader>Desktop update</SectionHeader>
                  <ListRow
                    title={update.availableVersion ? `Forge ${update.availableVersion}` : "Desktop release"}
                    subtitle={update.message ?? (update.checkedAt ? `Checked ${new Date(update.checkedAt).toLocaleString()}` : "Not checked yet")}
                    trailing={<Badge label={updateLabel(update.phase)} tone={update.phase === "available" ? "accent" : update.phase === "error" ? "warn" : "neutral"} />}
                    showSeparator={false}
                  />
                  {update.body ? (
                    <Text style={[type.sub, styles.releaseBody, { color: tokens.ink3 }]} numberOfLines={5}>
                      {update.body}
                    </Text>
                  ) : null}
                  <Button
                    label={
                      update.phase === "available"
                        ? "Install and relaunch"
                        : update.phase === "installing"
                          ? "Installing…"
                          : "Check for update"
                    }
                    onPress={update.phase === "available" ? install : checkUpdate}
                    loading={update.phase === "checking" || update.phase === "installing"}
                    disabled={update.phase === "checking" || update.phase === "installing"}
                    variant={update.phase === "available" ? "primary" : "secondary"}
                  />
                </View>
              ) : null}

              <View>
                <SectionHeader>Support</SectionHeader>
                <Text style={[type.sub, styles.supportNote, { color: tokens.ink3 }]}>
                  The copied summary uses a strict whitelist and omits the host name, connection
                  details, tokens, workspace content, logs, prompts, and daemon-provided free text.
                  Run <Text style={[type.code, { color: tokens.ink }]}>forge doctor</Text> on the host
                  for provider, bridge, and live connectivity probes.
                </Text>
                <View style={styles.actions}>
                  <Button label="Copy sanitized summary" onPress={copySummary} variant="secondary" />
                  <Button
                    label="Refresh diagnostics"
                    onPress={() => void refresh()}
                    loading={query.isFetching}
                    variant="ghost"
                  />
                </View>
              </View>
            </>
          ) : null}
        </Screen>
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: {
    width: "100%",
    maxWidth: 760,
    alignSelf: "center",
    gap: space.space24,
    paddingHorizontal: space.space16,
    paddingTop: space.space24,
    paddingBottom: space.space48,
  },
  header: { gap: space.space8 },
  fix: { paddingHorizontal: space.space16, paddingBottom: space.space12 },
  releaseBody: { paddingHorizontal: space.space16, paddingBottom: space.space12 },
  supportNote: { lineHeight: 20 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
