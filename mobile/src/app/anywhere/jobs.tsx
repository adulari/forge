// Forge Anywhere — the remote jobs this device queued for an offline host (a "Queue remote job"
// from the new-session sheet). Each is an encrypted command the host runs when it next connects;
// its contents are sealed, so a row shows where and when it was queued and what came back.
//
// This screen used to list the prototype's MockAnywhereClient seed jobs — demo rows no host had
// ever received — while the jobs actually queued from this phone (the hub counts them) never
// appeared here.
import { ListTodo } from "lucide-react-native";
import { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { EmptyState } from "../../components/ds/EmptyState";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/AnywhereProvider";
import type { PendingRemoteJob } from "../../lib/anywhereJobs";
import { goBackOr } from "../../lib/nav";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

function jobState(job: PendingRemoteJob, nowMs: number): string {
  if (job.result?.status === "success") return "ran on the host";
  if (job.result?.status === "error") {
    return job.result.retryable ? `failed (${job.result.code}) — can be retried` : `failed (${job.result.code})`;
  }
  if (job.expiresAtMs != null && job.expiresAtMs <= nowMs) return "expired before the host connected";
  return job.commandId ? "delivered · waiting for the host" : "queued on this device";
}

function queuedAt(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export default function AnywhereJobsScreen() {
  const tokens = useTokens();
  const anywhere = useAnywhere();
  const [now] = useState(() => Date.now());
  const hostName = (id: string) => anywhere.hosts.find((host) => host.id === id)?.name ?? `host ${id.slice(0, 8)}`;
  const jobs = [...anywhere.remoteJobs].sort((a, b) => b.createdAtMs - a.createdAtMs);

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <BackLink label="Anywhere" onPress={() => goBackOr("/anywhere")} />
      <Text style={[type.title, { color: tokens.ink }]}>Remote jobs</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>
        Sessions you started while a host was offline. The host runs each one when it reconnects.
      </Text>
      {jobs.length === 0 ? (
        <EmptyState icon={ListTodo} message="Nothing queued. Starting a session on an offline host queues it here." />
      ) : (
        <View>
          {jobs.map((job, index) => (
            <ListRow
              key={job.localId}
              title={hostName(job.hostId)}
              subtitle={`${jobState(job, now)} · queued ${queuedAt(job.createdAtMs)}`}
              showSeparator={index < jobs.length - 1}
            />
          ))}
        </View>
      )}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space12 },
});
