import { CheckCircle2, Clock3, Laptop, RefreshCw, XCircle } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { BackLink } from "../../components/ds/BackLink";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Input } from "../../components/ds/Input";
import { ListRow } from "../../components/ds/ListRow";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { useAnywhere } from "../../lib/AnywhereProvider";
import type { PendingRemoteJob } from "../../lib/anywhereJobs";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export default function AnywhereJobsScreen() {
  const anywhere = useAnywhere();
  const tokens = useTokens();
  const [hostId, setHostId] = useState(anywhere.hosts[0]?.id ?? "");
  const [cwd, setCwd] = useState("");
  const [title, setTitle] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const queue = async () => {
    setSubmitting(true);
    try {
      await anywhere.queueRemoteJob({
        hostId,
        cwd: cwd.trim() || undefined,
        title: title.trim() || undefined,
        worktree: true,
      });
      setTitle("");
    } catch { /* Provider exposes the durable queued state and safe error banner. */ }
    finally { setSubmitting(false); }
  };

  return (
    <Screen scroll keyboardAvoiding contentContainerStyle={styles.content}>
      <BackLink label="Forge Anywhere" />
      <View style={styles.hero}>
        <Text style={[type.title, { color: tokens.ink }]}>Queued remote jobs</Text>
        <Text style={[type.body, { color: tokens.ink2 }]}>Create a session on an enrolled host. The request is encrypted before it enters the offline queue.</Text>
      </View>
      {anywhere.error ? <Banner tone="warn" message={anywhere.error} /> : null}
      <SectionHeader>Destination</SectionHeader>
      <Card padded={false}>
        {anywhere.hosts.map((host, index) => (
          <ListRow
            key={host.id}
            title={host.name}
            subtitle={host.id === hostId ? "Selected" : (host.last_heartbeat_at ? "Recently connected" : "Offline jobs supported")}
            leading={<Laptop size={20} color={host.id === hostId ? tokens.accent : tokens.ink2} />}
            onPress={() => setHostId(host.id)}
            showSeparator={index < anywhere.hosts.length - 1}
          />
        ))}
      </Card>
      <Card style={styles.form}>
        <Input label="Working directory (optional)" value={cwd} onChangeText={setCwd} autoCapitalize="none" autoCorrect={false} placeholder="/path/on/host" />
        <Input label="Session title (optional)" value={title} onChangeText={setTitle} />
        <Button label="Queue encrypted job" onPress={() => void queue()} disabled={!hostId} loading={submitting} fullWidth />
        <Text style={[type.meta, { color: tokens.ink3 }]}>Paths and titles are ciphertext. The service sees only routing ids, time, and size.</Text>
      </Card>
      <View style={styles.sectionRow}>
        <SectionHeader>Recent jobs</SectionHeader>
        <Button label="Refresh" variant="ghost" icon={<RefreshCw size={16} color={tokens.ink2} />} onPress={() => void anywhere.refreshRemoteJobs()} />
      </View>
      <Card padded={false}>
        {anywhere.remoteJobs.length === 0 ? (
          <View style={styles.empty}><Text style={[type.sub, { color: tokens.ink3 }]}>No queued jobs on this device.</Text></View>
        ) : anywhere.remoteJobs.map((job, index) => (
          <ListRow
            key={job.localId}
            title={jobLabel(job)}
            subtitle={`${job.hostId.slice(0, 8)} · ${job.localId.slice(0, 8)}`}
            leading={jobIcon(job, tokens.accent, tokens.danger, tokens.ink3)}
            showSeparator={index < anywhere.remoteJobs.length - 1}
          />
        ))}
      </Card>
    </Screen>
  );
}

function jobLabel(job: PendingRemoteJob): string {
  if (job.result?.status === "success") return "Completed";
  if (job.result?.status === "error") return `Failed · ${job.result.code.replaceAll("_", " ")}`;
  if (job.commandId) return "Waiting for host";
  return "Queued on this device";
}

function jobIcon(job: PendingRemoteJob, success: string, danger: string, neutral: string) {
  if (job.result?.status === "success") return <CheckCircle2 size={20} color={success} />;
  if (job.result?.status === "error") return <XCircle size={20} color={danger} />;
  return <Clock3 size={20} color={neutral} />;
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space16 },
  hero: { gap: space.space8, maxWidth: 720 },
  form: { gap: space.space12 },
  sectionRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  empty: { padding: space.space16 },
});
