// Shells: the agent's background jobs (dev servers, watchers) and a live tail of the one you pick.
import { Terminal as TerminalIcon } from "lucide-react-native";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { useJobTail, useSessionJobs } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { stripAnsi } from "./checksModel";
import { rowBackground } from "./rowStyle";

export default function ShellsPanel({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const jobs = useSessionJobs(sessionId, true, 2000);
  const list = useMemo(() => jobs.data?.jobs ?? [], [jobs.data]);
  const [picked, setPicked] = useState<number | null>(null);
  const selected = list.find((job) => job.id === picked) ?? list.find((job) => job.running) ?? list[0] ?? null;
  const tail = useJobTail(sessionId, selected?.id ?? null, selected != null, 200, selected?.running ? 1500 : false);
  const scroller = useRef<ScrollView>(null);
  const text = useMemo(() => stripAnsi(tail.data?.text ?? ""), [tail.data?.text]);
  useEffect(() => {
    scroller.current?.scrollToEnd({ animated: false });
  }, [text]);

  if (jobs.isError) return <EmptyState icon={TerminalIcon} message="This daemon cannot list background shells yet. Update Forge to see them here." />;
  if (!jobs.isPending && list.length === 0) {
    return <EmptyState icon={TerminalIcon} message="No background shells. When the agent starts a dev server or a watcher, it shows up here with its live output." />;
  }
  return (
    <View style={styles.root}>
      <ScrollView style={[styles.list, { borderRightColor: tokens.hairline }]}>
        {list.map((job) => (
          <Pressable
            key={job.id}
            onPress={() => setPicked(job.id)}
            accessibilityRole="button"
            accessibilityState={{ selected: selected?.id === job.id }}
            style={({ hovered }: { hovered?: boolean; pressed: boolean }) => [styles.job, { backgroundColor: rowBackground(tokens, { hovered, selected: selected?.id === job.id }) }]}
          >
            <View style={[styles.dot, { backgroundColor: job.running ? tokens.success : "transparent", borderColor: job.running ? tokens.success : tokens.ink4 }]} />
            <View style={styles.jobText}>
              <Text numberOfLines={1} style={[styles.command, { color: tokens.ink }]}>
                {job.command}
              </Text>
              <Text numberOfLines={1} style={[styles.meta, { color: tokens.ink4 }]}>
                {`#${job.id} · ${job.status}`}
              </Text>
            </View>
          </Pressable>
        ))}
      </ScrollView>
      <ScrollView ref={scroller} style={styles.tail} contentContainerStyle={styles.tailBody}>
        <Text selectable style={[styles.out, { color: tokens.ink2 }]}>
          {text || (tail.isPending ? "" : "No output yet.")}
        </Text>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, flexDirection: "row", minHeight: 0 },
  list: { width: 240, borderRightWidth: StyleSheet.hairlineWidth },
  job: { minHeight: 36, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12 },
  dot: { width: 7, height: 7, borderRadius: 3.5, borderWidth: 1 },
  jobText: { flex: 1, minWidth: 0 },
  command: { fontFamily: monoFamily.regular, fontSize: 11 },
  meta: { fontFamily: sansFamilyFor(400), fontSize: 10 },
  tail: { flex: 1, minWidth: 0 },
  tailBody: { padding: space.space12 },
  out: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 16 },
});
