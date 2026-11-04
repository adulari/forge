// Background shell jobs of a session: the list, and the tail of the one you open. Read-only
// (stopping a job stays in the TUI's /jobs and the model's shell_job tool).
import React, { useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { useJobTail, useSessionJobs } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { jobAge, orderJobs } from "./jobsModel";

export function JobsSheet({
  visible,
  onClose,
  sessionId,
}: {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
}) {
  const tokens = useTokens();
  const [open, setOpen] = useState<number | null>(null);
  const jobs = useSessionJobs(sessionId, visible);
  const tail = useJobTail(sessionId, open, visible && open != null);
  const rows = orderJobs(jobs.data?.jobs ?? []);
  const now = Math.floor((jobs.dataUpdatedAt || 0) / 1000);
  const close = () => {
    setOpen(null);
    onClose();
  };

  return (
    <Sheet visible={visible} onClose={close} accessibilityLabel="Background jobs" snapPoints={[0.7]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Background jobs</Text>
        {open != null ? (
          <View style={styles.tail}>
            <Button label="Back to jobs" variant="secondary" onPress={() => setOpen(null)} />
            <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
              job {open} · {tail.data?.status ?? "loading"}
            </Text>
            <Text selectable style={[styles.log, { color: tokens.ink2 }]}>
              {tail.error ? tail.error.message : (tail.data?.text ?? "")}
            </Text>
          </View>
        ) : jobs.error ? (
          <Text style={[typeScale.sub, { color: tokens.danger }]}>{jobs.error.message}</Text>
        ) : rows.length === 0 ? (
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
            {jobs.isLoading ? "Loading…" : "No background jobs in this session's workspace."}
          </Text>
        ) : (
          rows.map((job, index) => (
            <ListRow
              key={job.id}
              title={job.command}
              subtitle={`${job.status} · ${jobAge(job.started_at, now)}`}
              onPress={() => setOpen(job.id)}
              showSeparator={index < rows.length - 1}
            />
          ))
        )}
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.space16, gap: space.space12 },
  tail: { gap: space.space8 },
  log: { fontFamily: monoFamily.regular, fontSize: 11.5, lineHeight: 16 },
});
